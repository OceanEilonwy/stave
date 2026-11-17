//! "One mistake produces one error" (grammar, "Tooling and editors"):
//! inject a single typical mistake into every example project, at several
//! places, and check that the pipeline reports exactly one diagnostic, on
//! the line of the mistake. Everything that depends on the broken item is
//! skipped silently; everything else still checks clean.
//!
//! The mistakes are the ones a beginner makes while typing: a value left
//! out, a misspelt name, a misspelt field, a unit that isn't one, and a
//! forgotten `)`. By default each is tried at the first, middle and last
//! place it fits in each example; set `STAVE_MISTAKES=all` to try every
//! place.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use stave_lang::render_diagnostics;
use stave_modules::MemoryFiles;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.stave` file under `examples/`, keyed by its path relative to
/// the workspace, in sorted order.
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

/// The example folders (directly under `examples/`).
fn examples() -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(workspace().join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_dir())
        .map(|path| path.strip_prefix(workspace()).unwrap().to_path_buf())
        .collect();
    folders.sort();
    folders
}

/// A line of a file with its position.
struct Line<'a> {
    file: usize,
    number: usize,
    text: &'a str,
    /// The line is a whole entry: no bracket is open before it or after
    /// it, and it isn't inside a `roles { }` block.
    whole_entry: bool,
}

/// The code of a line without its string contents and trailing comment,
/// for counting brackets.
fn code_of(line: &str) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '/' if !in_string && chars.peek() == Some(&'/') => break,
            _ if in_string => {}
            _ => out.push(c),
        }
    }
    out
}

fn lines(files: &[(PathBuf, String)]) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    for (index, (_, text)) in files.iter().enumerate() {
        let mut depth: i32 = 0;
        let mut blocks: Vec<bool> = Vec::new(); // true for a `roles {` block
        let mut comment = false;
        for (number, line) in text.lines().enumerate() {
            let code = code_of(line);
            let trimmed = code.trim();
            if trimmed.starts_with("/*") {
                comment = !trimmed.contains("*/");
            }
            let starts_at_zero = depth == 0;
            for c in code.chars() {
                match c {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '{' => blocks.push(trimmed.starts_with("roles")),
                    '}' => {
                        blocks.pop();
                    }
                    _ => {}
                }
            }
            out.push(Line {
                file: index,
                number: number + 1,
                text: line,
                whole_entry: starts_at_zero
                    && depth == 0
                    && !comment
                    && !blocks.last().copied().unwrap_or(false)
                    && !trimmed.starts_with("roles"),
            });
        }
    }
    out
}

/// Item names declared in the files, for misspelling references.
fn declared_names(files: &[(PathBuf, String)]) -> BTreeSet<String> {
    let keywords = [
        "part", "region", "let", "param", "stock", "joint", "hardware", "assembly", "profile",
    ];
    let mut names = BTreeSet::new();
    for (_, text) in files {
        for line in text.lines() {
            let mut words = line.split_whitespace();
            if let (Some(keyword), Some(name)) = (words.next(), words.next())
                && keywords.contains(&keyword)
            {
                let name: String = name
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    names.insert(name);
                }
            }
        }
    }
    names
}

/// One kind of mistake: where it fits, and the line with the mistake made.
struct Mistake {
    name: &'static str,
    apply: fn(&str, &BTreeSet<String>) -> Option<String>,
}

/// `key: value` as a whole entry, split into indent, key and the rest.
fn field(line: &str) -> Option<(&str, &str, &str)> {
    let indent = &line[..line.len() - line.trim_start().len()];
    let rest = line.trim_start();
    let colon = rest.find(':')?;
    let key = &rest[..colon];
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return None;
    }
    let value = rest[colon + 1..].trim();
    let value = code_of(value);
    let value = value.trim();
    if value.is_empty() || value.ends_with([',', '+', '-', '*', '/', '(', '[', '{']) {
        return None;
    }
    Some((indent, key, &rest[colon + 1..]))
}

const MISTAKES: &[Mistake] = &[
    Mistake {
        name: "a value left out",
        apply: |line, _| {
            let (indent, key, _) = field(line)?;
            Some(format!("{indent}{key}:"))
        },
    },
    Mistake {
        name: "a misspelt field",
        apply: |line, _| {
            let (indent, key, rest) = field(line)?;
            Some(format!("{indent}{key}x:{rest}"))
        },
    },
    Mistake {
        name: "a misspelt name",
        apply: |line, names| {
            let code = code_of(line);
            let trimmed = code.trim_start();
            // Not the declaration itself.
            let first = trimmed.split_whitespace().next().unwrap_or("");
            if [
                "part",
                "region",
                "let",
                "param",
                "stock",
                "joint",
                "hardware",
                "assembly",
                "profile",
                "kind",
                "component",
                "recipe",
                "scene",
                "require",
                "import",
            ]
            .contains(&first)
            {
                return None;
            }
            // The first use of a declared name followed by `.`, `[` or `)`.
            let bytes = code.as_bytes();
            let mut start = 0;
            while start < bytes.len() {
                let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                if is_word(bytes[start]) && (start == 0 || !is_word(bytes[start - 1])) {
                    let mut end = start;
                    while end < bytes.len() && is_word(bytes[end]) {
                        end += 1;
                    }
                    let word = &code[start..end];
                    let before_dot = start > 0 && bytes[start - 1] == b'.';
                    if names.contains(word)
                        && !before_dot
                        && matches!(bytes.get(end), Some(b'.' | b'[' | b')' | b','))
                    {
                        let offset = line.find(&code[..end]).unwrap_or(0);
                        let at = offset + end;
                        return Some(format!("{}q{}", &line[..at], &line[at..]));
                    }
                    start = end;
                } else {
                    start += 1;
                }
            }
            None
        },
    },
    Mistake {
        name: "a unit that isn't one",
        apply: |line, _| {
            let code = code_of(line);
            for unit in ["mm", "deg", "in"] {
                let mut search = 0;
                while let Some(found) = code[search..].find(unit) {
                    let at = search + found;
                    let after = code.as_bytes().get(at + unit.len());
                    let digit_before = at > 0 && code.as_bytes()[at - 1].is_ascii_digit();
                    if digit_before && !after.is_some_and(|b| b.is_ascii_alphanumeric()) {
                        let offset = line.find(code.trim_start()).unwrap_or(0)
                            - (code.len() - code.trim_start().len());
                        let at = offset + at;
                        return Some(format!("{}qq{}", &line[..at], &line[at + unit.len()..]));
                    }
                    search = at + unit.len();
                }
            }
            None
        },
    },
    Mistake {
        name: "a forgotten ')'",
        apply: |line, _| {
            let code = code_of(line);
            let trimmed = code.trim_end();
            if !trimmed.ends_with(')') || trimmed.ends_with("()") {
                return None;
            }
            // The last `)` of the code, not of a trailing comment.
            let at = line.find(trimmed.trim_start())? + trimmed.trim_start().len() - 1;
            Some(format!("{}{}", &line[..at], &line[at + 1..]))
        },
    },
];

fn open(files: &[(PathBuf, String)], example: &Path) -> stave_app::Opened {
    let mut memory = MemoryFiles::new();
    for (path, text) in files {
        memory.insert(Path::new("/").join(path), text.as_str());
    }
    stave_app::open_with(&Path::new("/").join(example), &memory).expect("the example loads")
}

/// The places to try: first, middle and last, or all of them.
fn places<T>(candidates: &[T]) -> Vec<&T> {
    if std::env::var("STAVE_MISTAKES").is_ok_and(|v| v == "all") || candidates.len() <= 3 {
        return candidates.iter().collect();
    }
    let last = candidates.len() - 1;
    [0, last / 2, last]
        .into_iter()
        .filter_map(|index| candidates.get(index))
        .collect()
}

#[test]
fn one_mistake_in_any_example_is_exactly_one_error_on_its_line() {
    let files = example_files();
    let mut tried = 0;
    let mut failures = Vec::new();
    for example in examples() {
        // The files of this example and of the folders it may import.
        let clean = open(&files, &example);
        assert!(
            clean.diagnostics.is_empty(),
            "{} isn't clean to start with",
            example.display()
        );
        let own: Vec<(PathBuf, String)> = files
            .iter()
            .filter(|(path, _)| path.starts_with(&example))
            .cloned()
            .collect();
        let names = declared_names(&own);
        let all_lines = lines(&own);
        for mistake in MISTAKES {
            let candidates: Vec<(&Line<'_>, String)> = all_lines
                .iter()
                .filter(|line| line.whole_entry)
                .filter_map(|line| (mistake.apply)(line.text, &names).map(|text| (line, text)))
                .collect();
            assert!(
                !candidates.is_empty() || mistake.name == "a forgotten ')'",
                "{}: no place for {}",
                example.display(),
                mistake.name
            );
            for (line, mutated) in places(&candidates) {
                tried += 1;
                let (path, text) = &own[line.file];
                let new_text: String = text
                    .lines()
                    .enumerate()
                    .map(|(index, original)| {
                        if index + 1 == line.number {
                            mutated.as_str()
                        } else {
                            original
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    + "\n";
                let changed: Vec<(PathBuf, String)> = files
                    .iter()
                    .map(|(p, t)| {
                        if p == path {
                            (p.clone(), new_text.clone())
                        } else {
                            (p.clone(), t.clone())
                        }
                    })
                    .collect();
                let opened = open(&changed, &example);
                let where_ = format!(
                    "{} in {}:{}: `{}` -> `{}`",
                    mistake.name,
                    path.display(),
                    line.number,
                    line.text.trim(),
                    mutated.trim()
                );
                let rendered = render_diagnostics(&opened.sources, &opened.diagnostics);
                match opened.diagnostics.as_slice() {
                    [only] => {
                        let at = common::line(&opened, only);
                        let file = only
                            .primary()
                            .and_then(|label| opened.sources.get(label.file))
                            .map(|f| f.path().to_owned())
                            .unwrap_or_default();
                        if at != line.number || !file.ends_with(&*path.to_string_lossy()) {
                            failures.push(format!("{where_}: reported elsewhere:\n{rendered}"));
                        } else if only.help.is_none() {
                            failures.push(format!("{where_}: no help line:\n{rendered}"));
                        }
                    }
                    _ => failures.push(format!(
                        "{where_}: {} diagnostics:\n{rendered}",
                        opened.diagnostics.len()
                    )),
                }
            }
        }
    }
    assert!(tried >= 100, "only {tried} mistakes tried");
    assert!(
        failures.is_empty(),
        "{} of {tried} mistakes broke the rule:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// The recoveries the sweep above needed, one design each
// ---------------------------------------------------------------------------

#[test]
fn a_syntax_error_in_the_project_block_is_one_error() {
    let opened = common::open_files(&[(
        "project.stave",
        "stave 1\n\nproject \"Test\" {\n  units:\n}\n",
    )]);
    common::only_error(&opened, "missing-value", "'units' has no value");
}

#[test]
fn a_size_decided_by_a_joint_with_its_own_error_is_not_also_undecided() {
    // The let is broken, so the dado can't be worked out, so nothing
    // decides the shelf's length: only the let's error is reported.
    let opened = common::open(
        "let depth = 9qq
part side[left, right] : pine_19 {
  length: 400mm
  width:  300mm
  place {
    orient(length: up, width: back)
    against(base.top)
    spaced(ends, from: base.left, to: base.right)
    flush(base.front)
  }
}
part shelf : pine_19 {
  length: fit
  width:  300mm
  place {
    against(base.top, gap: 100mm)
    center(base, across)
    flush(base.front)
  }
}
joint housings : dado(shelf, into: side[*], depth: depth)",
    );
    common::only_error(
        &opened,
        "unknown-unit",
        "unknown unit 'qq' (expected one of mm, cm, m, in, ft, deg)",
    );
}

#[test]
fn a_bracket_forgotten_in_a_place_block_is_one_error_on_its_line() {
    let opened = common::open(
        "part block : pine_19 {\n  length: 100mm\n  width:  50mm\n  place {\n    flush(base.left, inset: 10mm\n    against(base.top)\n    flush(base.front)\n  }\n}",
    );
    let diagnostic =
        common::the_diagnostic(&opened, "unclosed-bracket", "this \"(\" is never closed");
    assert_eq!(common::body_line(&opened, diagnostic), 5);
    // The help names the line that lacks the `)`, not the end of the block.
    let line = common::line(&opened, diagnostic);
    assert_eq!(
        diagnostic.help.as_deref(),
        Some(format!("add \")\" at the end of line {line}").as_str())
    );
}

#[test]
fn a_bracket_forgotten_before_a_doc_comment_is_one_error() {
    let opened = common::open("let a = max_of(1mm, 2mm\n\n/// The next item.\nlet b = 1mm");
    common::only_error(&opened, "unclosed-bracket", "this \"(\" is never closed");
}

#[test]
fn a_component_param_with_a_syntax_error_is_skipped_where_it_is_given_and_read() {
    let opened = common::open(
        "component Box {\n  param height = 50qq\n}\nassembly box : Box {\n  height: 60mm\n}\nlet h = box.height",
    );
    common::only_error(
        &opened,
        "unknown-unit",
        "unknown unit 'qq' (expected one of mm, cm, m, in, ft, deg)",
    );
}

#[test]
fn a_misspelt_recipe_tools_field_is_one_error() {
    let opened = common::open(
        "recipe r : notch {\n  toolz: [chisel]\n  cut {\n    chop(waste, with: chisel)\n  }\n}",
    );
    common::only_error(&opened, "unknown-field", "recipe 'r' has no field 'toolz'");
}
