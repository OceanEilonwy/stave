//! The standing rule (`CLAUDE.md`, "No kind in the core"): the core crates
//! never name a joint or hardware kind, a member of one, or a field or
//! param only a kind has, in code or in a string. Behaviour comes from the
//! language; a behaviour a kind needs is a new language primitive with a
//! spec entry, never a special case.
//!
//! [`the_core_names_no_kind`] reads the words to look for out of `std/`
//! (every kind's name in `std/hardware` and `std/joinery`, and every
//! member, param and field of a hardware kind), adds a short fixed list
//! of catalogue words, and takes away [`ALLOWED_WORDS`]: words that are the
//! language's own vocabulary or plain English, each with its reason (and
//! each must still be a word of `std/`, so the list can't go stale). It
//! then reads every `src/**/*.rs` of the core crates, skipping comments and
//! test modules (everything after a `#[cfg(test)]` line, and a module's own
//! `tests.rs`), and fails naming the file, line and word of every hit.
//!
//! A source word is compared in snake case (`CupParts` is `cup_parts`), as
//! a whole and by its `_`-separated parts, so `cup_parts` is a hit for
//! `cup` and `side_mount_slide` for itself, but `mount_face` is not a hit
//! for `side_mount_slide`.
//!
//! The other half of the rule, that no check lets a pairing off, is held
//! by behaviour, not by reading source: `app/fit` and
//! `tests/broken/hinge_link_in_wood`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The core crates the rule holds for.
const CORE: [&str; 8] = [
    "stave-lang",
    "stave-schema",
    "stave-modules",
    "stave-model",
    "stave-solve",
    "stave-geom",
    "stave-check",
    "stave-scene",
];

/// Catalogue words that are not a kind's name, member or entry in `std/`
/// but name one family of hardware or joint all the same. (`butt` is not
/// here: it is the contact a joint's parts make, end to face,
/// `Contact::Butt` in the grammar's joint pairs; `butt_hinge` is still a
/// hit, as a kind's name. Nor is `leaves`, an English verb.)
const CATALOGUE_WORDS: [&str; 13] = [
    "cup",
    "euro",
    "leaf",
    "knuckle",
    "linkage",
    "four_bar",
    "chidori",
    "ari",
    "kanawa",
    "base_overlay",
    "overlay_adjustment",
    "hinge_overlay",
    "slide_gap",
];

/// Words read out of `std/` that are the language's own vocabulary, not a
/// kind's: the core may use them. Every entry says why; review reads this
/// list (work pack "generalise kinds", stage 8).
const ALLOWED_WORDS: [(&str, &str); 11] = [
    (
        "species",
        "a stock's species word, a closed vocabulary of the language, and a fastener base's optional entry (what a placed fastener is made of)",
    ),
    (
        "length",
        "a part's axis and a box's size in the language (`length:`), and a fastener base's required entry",
    ),
    (
        "diameter",
        "the argument of `cylinder(diameter:)`, `drill(diameter:)` and `round(diameter:)`, and a fastener base's required entry",
    ),
    (
        "travel",
        "the language's drive `travel(member, from:, to:)` and a scene's travel value",
    ),
    (
        "bar",
        "the language's shape `bar(from:, to:, thickness:, along:)`",
    ),
    (
        "dowel",
        "a stock form (`form: dowel`), a closed vocabulary of the language",
    ),
    (
        "envelope",
        "a fastener base's entry and the language's type `Envelope` (`box(length:, ...)`)",
    ),
    (
        "socket",
        "the cut operation `dovetail_socket` and the socket a dovetail tail goes into, in its messages",
    ),
    ("centres", "plain English: the centres of holes and shapes"),
    (
        "projection",
        "the scene camera's projection (orthographic or perspective), not a pull's stand-off",
    ),
    (
        "clearance",
        "plain English: the gap the clearance check measures (`stave_geom::clearance`), not a slide's catalogue entry",
    ),
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every `.stave` file under a folder, sorted.
fn stave_files(folder: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap_or_else(|error| panic!("can't read {}: {error}", folder.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "stave"))
        .collect();
    found.sort();
    found
}

/// Every `.rs` file under a folder, sorted, without a module's own
/// `tests.rs` (a test module kept in its own file).
fn rust_files(folder: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = fs::read_dir(folder)
        .unwrap_or_else(|error| panic!("can't read {}: {error}", folder.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs")
            && path.file_name().is_some_and(|name| name != "tests.rs")
        {
            out.push(path);
        }
    }
}

/// The first word of a line after `keyword ` (`kind euro_hinge : hinge`).
fn after(line: &str, keyword: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix(keyword)?.strip_prefix(' ')?;
    let word: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    (!word.is_empty()).then_some(word)
}

/// The words to look for, read out of `std/`: every kind's name, and every
/// member, param and field of a hardware kind.
fn std_words() -> BTreeSet<String> {
    let mut words = BTreeSet::new();
    for folder in ["std/hardware", "std/joinery"] {
        for file in stave_files(&workspace().join(folder)) {
            let text = fs::read_to_string(&file).unwrap();
            let mut hardware = false;
            for line in text.lines() {
                if let Some(name) = after(line, "kind") {
                    words.insert(name);
                    let base = line.split(':').nth(1).unwrap_or("").trim();
                    hardware = ["hinge", "slide", "pull", "fastener"]
                        .iter()
                        .any(|b| base.starts_with(b));
                    continue;
                }
                if line.starts_with('}') {
                    hardware = false;
                }
                if !hardware {
                    continue;
                }
                for keyword in ["member", "param"] {
                    if let Some(name) = after(line, keyword) {
                        words.insert(name);
                    }
                }
                // A field: `  name: value` at the kind's own indent.
                if let Some(rest) = line.strip_prefix("  ")
                    && !rest.starts_with(' ')
                {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() && rest[name.len()..].trim_start().starts_with(':') {
                        words.insert(name);
                    }
                }
            }
        }
    }
    words
}

/// A Rust source with its comments blanked out (string and character
/// literals kept, so a word in a message is still seen), line by line.
fn without_comments(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
        } else if c == 'r' && (next == Some('"') || next == Some('#')) {
            // A raw string: r"..." or r#"..."#.
            let mut j = i + 1;
            let mut hashes = 0;
            while chars.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if chars.get(j) != Some(&'"') {
                out.push(c);
                i += 1;
                continue;
            }
            out.extend(&chars[i..=j]);
            j += 1;
            while let Some(&d) = chars.get(j) {
                out.push(d);
                j += 1;
                if d == '"' && (0..hashes).all(|k| chars.get(j + k) == Some(&'#')) {
                    out.extend(std::iter::repeat_n('#', hashes));
                    j += hashes;
                    break;
                }
            }
            i = j;
        } else if c == '"' {
            out.push(c);
            i += 1;
            while i < chars.len() {
                let d = chars[i];
                out.push(d);
                i += 1;
                if d == '\\' {
                    if let Some(&e) = chars.get(i) {
                        out.push(e);
                        i += 1;
                    }
                } else if d == '"' {
                    break;
                }
            }
        } else if c == '\'' {
            // A character literal ('x', '\n', '"'), or a lifetime ('a).
            let literal = match (next, chars.get(i + 2), chars.get(i + 3)) {
                (Some('\\'), _, _) => chars[i + 1..]
                    .iter()
                    .skip(2)
                    .position(|d| *d == '\'')
                    .map(|end| end + 4),
                (Some(_), Some('\''), _) => Some(3),
                _ => None,
            };
            let take = literal.unwrap_or(1);
            out.extend(chars.iter().skip(i).take(take));
            i += take;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out.lines().map(str::to_owned).collect()
}

/// A word in snake case: `CupParts` is `cup_parts`, `MAX_PLATE` is
/// `max_plate`.
fn snake(word: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = word.chars().collect();
    for (index, c) in chars.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|p| chars.get(p));
        let next = chars.get(index + 1);
        let boundary = c.is_uppercase()
            && previous.is_some_and(|p| {
                p.is_lowercase()
                    || p.is_ascii_digit()
                    || (p.is_uppercase() && next.is_some_and(|n| n.is_lowercase()))
            });
        if boundary {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// Does a source word (snake case) contain `word` as a whole or as a run
/// of its `_`-separated parts?
fn contains_word(token: &str, word: &str) -> bool {
    format!("_{token}_").contains(&format!("_{word}_"))
}

#[test]
fn the_core_names_no_kind() {
    let mut words = std_words();
    words.extend(CATALOGUE_WORDS.iter().map(|w| (*w).to_owned()));
    for (word, _) in ALLOWED_WORDS {
        assert!(
            words.remove(word),
            "`{word}` is allowed but is no longer a word of std/: take it off the list"
        );
    }
    // The words must really come from std: a few that must be there.
    for expected in [
        "euro_hinge",
        "cup_inset",
        "plate_height",
        "dado",
        "drawer_member",
    ] {
        assert!(words.contains(expected), "std/ no longer has {expected}");
    }
    let mut hits = Vec::new();
    for krate in CORE {
        let mut files = Vec::new();
        rust_files(
            &workspace().join("crates").join(krate).join("src"),
            &mut files,
        );
        for file in files {
            let source = fs::read_to_string(&file).unwrap();
            for (index, line) in without_comments(&source).iter().enumerate() {
                if line.trim_start().starts_with("#[cfg(test)]") {
                    break;
                }
                let tokens = line
                    .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .filter(|t| !t.is_empty())
                    .map(snake);
                for token in tokens {
                    for word in &words {
                        if contains_word(&token, word) {
                            let shown = file.strip_prefix(workspace()).unwrap_or(&file);
                            hits.push(format!(
                                "{}:{}: `{word}` (in `{token}`)",
                                shown.display(),
                                index + 1
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "the core names a kind, a member of one or a field only a kind has \
         (CLAUDE.md, \"No kind in the core\"); say it in the language instead:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_scanner_sees_code_and_strings_but_not_comments() {
    let lines = without_comments(
        "let cup = 1; // a cup\n/* cup\n cup */ let s = \"the cup\";\nlet c = '\"'; let d = \"// not a comment cup\";\nfn f<'a>(x: &'a str) {}\n",
    );
    assert_eq!(lines[0].trim_end(), "let cup = 1;");
    assert_eq!(lines[2].trim(), "let s = \"the cup\";");
    assert!(lines[3].contains("not a comment cup"));
    assert!(lines[4].contains("fn f<'a>"));
    assert_eq!(snake("CupParts"), "cup_parts");
    assert_eq!(snake("MAX_PLATE_NM"), "max_plate_nm");
    assert!(contains_word("cup_parts", "cup"));
    assert!(contains_word("side_mount_slide", "side_mount_slide"));
    assert!(!contains_word("mount_face", "side_mount_slide"));
    assert!(!contains_word("cupboard", "cup"));
}
