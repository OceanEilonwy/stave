//! Enforces `docs/test-coverage.md`, the map from every rule of the spec
//! and every diagnostic code to the tests that pin it down:
//!
//! - every row has a status: `covered`, `partial: <reason>` or
//!   `pending: <reason>`; covered and partial rows link a test;
//! - every link (`crate/file::test`, `broken/<folder>`) points at a test or
//!   broken design that exists;
//! - every code of the grammar's Diagnostics table, and every warning, has
//!   exactly one row, and its status agrees with the broken-design corpus:
//!   a covered code has a broken design that isn't pending, a pending code
//!   has none;
//! - every broken design is listed.
//!
//! So a new diagnostic code can't be added to the spec without a coverage
//! decision, and a renamed or deleted test can't leave a dangling link.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(workspace().join(path)).unwrap()
}

/// The cells of a table row, splitting on `|` outside backticks.
fn cells(row: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    for c in row
        .trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .chars()
    {
        match c {
            '`' => {
                in_code = !in_code;
                current.push(c);
            }
            '|' if !in_code => cells.push(std::mem::take(&mut current).trim().to_owned()),
            _ => current.push(c),
        }
    }
    cells.push(current.trim().to_owned());
    cells
}

/// The rows of every table in `text`, without header and separator rows.
fn rows(text: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut header = false;
    for line in text.lines() {
        if !line.starts_with('|') {
            header = true;
            continue;
        }
        if header {
            // The first row of a table is its header.
            header = false;
            continue;
        }
        if line.starts_with("|---") {
            continue;
        }
        out.push(cells(line));
    }
    out
}

/// The backticked words of a cell.
fn code_spans(cell: &str) -> Vec<&str> {
    cell.split('`').skip(1).step_by(2).collect()
}

/// What a broken design expects: its code, and whether it is pending.
fn broken_designs() -> BTreeMap<String, (String, bool)> {
    let mut designs = BTreeMap::new();
    for entry in std::fs::read_dir(workspace().join("tests/broken")).unwrap() {
        let folder = entry.unwrap().path();
        let text = std::fs::read_to_string(folder.join("project.stave")).unwrap();
        let mut lines = text.lines();
        let first = lines.next().unwrap();
        let rest = first
            .strip_prefix("// expect: error[")
            .or_else(|| first.strip_prefix("// expect: warning["))
            .unwrap();
        let code = rest.split(']').next().unwrap().to_owned();
        let pending = lines.next().is_some_and(|l| l.starts_with("// pending: "));
        let name = folder.file_name().unwrap().to_string_lossy().into_owned();
        designs.insert(name, (code, pending));
    }
    designs
}

/// Checks one link; returns what is wrong with it.
fn check_link(link: &str, designs: &BTreeMap<String, (String, bool)>) -> Result<(), String> {
    if let Some(folder) = link.strip_prefix("broken/") {
        return if designs.contains_key(folder) {
            Ok(())
        } else {
            Err(format!("no broken design '{folder}'"))
        };
    }
    let (path, test) = link
        .split_once("::")
        .ok_or_else(|| format!("'{link}' is not crate/file::test"))?;
    let (krate, file) = path
        .split_once('/')
        .ok_or_else(|| format!("'{link}' is not crate/file::test"))?;
    let candidates = [
        format!("crates/stave-{krate}/tests/{file}.rs"),
        format!("crates/stave-{krate}/src/{file}.rs"),
    ];
    let mut found_file = false;
    for candidate in &candidates {
        let Ok(text) = std::fs::read_to_string(workspace().join(candidate)) else {
            continue;
        };
        found_file = true;
        // The test function, right after its #[test] attribute.
        let mut previous = "";
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(&format!("fn {test}(")) && previous == "#[test]" {
                return Ok(());
            }
            if !trimmed.is_empty() {
                previous = trimmed;
            }
        }
    }
    if found_file {
        Err(format!("no test '{test}' in {}", candidates.join(" or ")))
    } else {
        Err(format!("no file {}", candidates.join(" or ")))
    }
}

/// `covered`, `partial: <reason>` or `pending: <reason>`.
fn check_status(status: &str) -> Result<&str, String> {
    if status == "covered" {
        return Ok("covered");
    }
    for kind in ["partial", "pending"] {
        if let Some(reason) = status.strip_prefix(&format!("{kind}: ")) {
            return if reason.trim().is_empty() {
                Err(format!("'{status}' needs a reason"))
            } else {
                Ok(kind)
            };
        }
    }
    Err(format!(
        "status '{status}' is not covered, partial: <reason> or pending: <reason>"
    ))
}

/// Every code of the grammar's Diagnostics table and its warnings line.
fn spec_codes() -> Vec<String> {
    let grammar = read("docs/grammar.md");
    let start = "| Layer | Code | Message |\n|---|---|---|\n";
    let from = grammar.find(start).expect("the Diagnostics table") + start.len();
    let rest = &grammar[from..];
    let table = &rest[..rest.find("\n\n").unwrap()];
    let mut codes: Vec<String> = table
        .lines()
        .map(|row| cells(row)[1].trim_matches('`').to_owned())
        .collect();
    let warnings = "Warnings and notes: ";
    let from = grammar.find(warnings).unwrap() + warnings.len();
    let rest = &grammar[from..];
    for word in rest[..rest.find(".\n").unwrap()].split(',') {
        let code = word.trim().trim_matches('`').to_owned();
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

fn map() -> (String, String) {
    let text = read("docs/test-coverage.md");
    let features = text.find("\n## Features").expect("a Features section");
    let diagnostics = text
        .find("\n## Diagnostics\n")
        .expect("a Diagnostics section");
    (
        text[features..diagnostics].to_owned(),
        text[diagnostics..].to_owned(),
    )
}

#[test]
fn every_feature_row_has_a_status_and_links_tests_that_exist() {
    let designs = broken_designs();
    let (features, _) = map();
    let mut problems = Vec::new();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let rows = rows(&features);
    assert!(rows.len() >= 250, "only {} feature rows", rows.len());
    for row in &rows {
        let [rule, tests, status] = row.as_slice() else {
            problems.push(format!("not three cells: {row:?}"));
            continue;
        };
        match check_status(status) {
            Ok(kind) => {
                *counts.entry(kind).or_default() += 1;
                let links = code_spans(tests);
                let refers = tests.starts_with("see [");
                if kind != "pending" && links.is_empty() && !refers {
                    problems.push(format!("'{rule}' is {kind} but links no test"));
                }
                for link in links {
                    if let Err(problem) = check_link(link, &designs) {
                        problems.push(format!("'{rule}': {problem}"));
                    }
                }
            }
            Err(problem) => problems.push(format!("'{rule}': {problem}")),
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
    eprintln!("feature rows: {counts:?}");
}

#[test]
fn every_diagnostic_code_has_one_row_that_agrees_with_the_broken_corpus() {
    let designs = broken_designs();
    let (_, diagnostics) = map();
    let mut problems = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut listed_designs: BTreeSet<String> = BTreeSet::new();
    for row in rows(&diagnostics) {
        let [code, _layer, status, broken, tests] = row.as_slice() else {
            problems.push(format!("not five cells: {row:?}"));
            continue;
        };
        let code = code.trim_matches('`').to_owned();
        if !seen.insert(code.clone()) {
            problems.push(format!("'{code}' has two rows"));
        }
        let kind = match check_status(status) {
            Ok(kind) => kind,
            Err(problem) => {
                problems.push(format!("'{code}': {problem}"));
                continue;
            }
        };
        for link in code_spans(broken).into_iter().chain(code_spans(tests)) {
            if let Err(problem) = check_link(link, &designs) {
                problems.push(format!("'{code}': {problem}"));
            }
        }
        let listed: Vec<&str> = code_spans(broken)
            .into_iter()
            .filter_map(|link| link.strip_prefix("broken/"))
            .collect();
        for folder in &listed {
            listed_designs.insert((*folder).to_owned());
            if let Some((expected, _)) = designs.get(*folder)
                && *expected != code
            {
                problems.push(format!(
                    "'{code}' lists broken/{folder}, which expects '{expected}'"
                ));
            }
        }
        let live = |folder: &&str| designs.get(*folder).is_some_and(|(_, pending)| !pending);
        match kind {
            "covered" | "partial" => {
                if !listed.iter().any(live) {
                    problems.push(format!(
                        "'{code}' is {kind} but lists no broken design that checks it"
                    ));
                }
                if code_spans(tests).is_empty() {
                    problems.push(format!("'{code}' is {kind} but links no test"));
                }
            }
            _ => {
                if let Some((folder, _)) = designs
                    .iter()
                    .find(|(_, (expected, pending))| *expected == code && !pending)
                {
                    problems.push(format!(
                        "'{code}' is pending, but broken/{folder} checks it: mark it covered"
                    ));
                }
            }
        }
    }
    for code in spec_codes() {
        if !seen.contains(&code) {
            problems.push(format!(
                "the spec's code '{code}' has no row in docs/test-coverage.md"
            ));
        }
    }
    for code in &seen {
        if !spec_codes().contains(code) {
            problems.push(format!("'{code}' is not a code of the spec"));
        }
    }
    for folder in designs.keys() {
        if !listed_designs.contains(folder) {
            problems.push(format!(
                "broken/{folder} is not listed in the Diagnostics table"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
