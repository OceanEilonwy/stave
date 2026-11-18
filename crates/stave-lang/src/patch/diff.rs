//! The smallest text edits between a file's old and new text.
//!
//! An editor that has the file open takes a patch as edits to its buffer,
//! not as a new text, so the cursor, the marks and the undo history stay
//! where they were. The edits are found by comparing lines (Myers' diff),
//! then trimming what each changed run of lines has in common at its start
//! and its end, so a changed number is one edit of a few characters.

use std::ops::Range;

/// One replacement in a file's old text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextEdit {
    /// The bytes of the old text replaced; empty for an insertion. Both
    /// ends are on character boundaries.
    pub range: Range<usize>,
    /// What replaces them; empty for a deletion.
    pub text: String,
}

/// Applies edits given last in the file first, as
/// [`PatchedFile::edits`](super::PatchedFile::edits) gives them, to the
/// text they were computed from. `None` if the edits are out of that order,
/// overlap, or fall outside the text or between the bytes of a character.
pub fn apply_text_edits(text: &str, edits: &[TextEdit]) -> Option<String> {
    let mut out = text.to_owned();
    let mut limit = text.len();
    for edit in edits {
        if edit.range.start > edit.range.end || edit.range.end > limit {
            return None;
        }
        if !out.is_char_boundary(edit.range.start) || !out.is_char_boundary(edit.range.end) {
            return None;
        }
        out.replace_range(edit.range.clone(), &edit.text);
        limit = edit.range.start;
    }
    Some(out)
}

/// The most edit steps the line comparison takes before it gives up and
/// replaces the rest as one run. Patches change a few lines, so this is
/// reached only by a rewrite of most of a large file.
const MOST_STEPS: usize = 4096;

/// The smallest edits that turn `old` into `new`, last in the file first.
pub(super) fn text_edits(old: &str, new: &str) -> Vec<TextEdit> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let old_starts = starts(&old_lines);
    let new_starts = starts(&new_lines);
    let mut edits: Vec<TextEdit> = changed_runs(&old_lines, &new_lines)
        .into_iter()
        .filter_map(|(old_run, new_run)| {
            let old_bytes = *old_starts.get(old_run.start)?..*old_starts.get(old_run.end)?;
            let new_bytes = *new_starts.get(new_run.start)?..*new_starts.get(new_run.end)?;
            trimmed(old, old_bytes, new.get(new_bytes)?)
        })
        .collect();
    edits.reverse();
    edits
}

/// The byte offset where each line starts, and the end of the text last.
fn starts(lines: &[&str]) -> Vec<usize> {
    let mut starts = Vec::with_capacity(lines.len() + 1);
    let mut at = 0;
    starts.push(at);
    for line in lines {
        at += line.len();
        starts.push(at);
    }
    starts
}

/// The edit replacing `old[range]` with `new`, less the characters the two
/// share at their start and their end; `None` when they are the same.
fn trimmed(old: &str, range: Range<usize>, new: &str) -> Option<TextEdit> {
    let replaced = old.get(range.clone())?;
    let prefix: usize = replaced
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let (replaced_rest, new_rest) = (replaced.get(prefix..)?, new.get(prefix..)?);
    let suffix: usize = replaced_rest
        .chars()
        .rev()
        .zip(new_rest.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let text = new_rest.get(..new_rest.len() - suffix)?;
    let range = range.start + prefix..range.end - suffix;
    if range.is_empty() && text.is_empty() {
        return None;
    }
    Some(TextEdit {
        range,
        text: text.to_owned(),
    })
}

/// The runs of lines that differ, as (old lines, new lines), in order.
fn changed_runs(old: &[&str], new: &[&str]) -> Vec<(Range<usize>, Range<usize>)> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let old_rest = old.get(prefix..).unwrap_or_default();
    let new_rest = new.get(prefix..).unwrap_or_default();
    let suffix = old_rest
        .iter()
        .rev()
        .zip(new_rest.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_middle = old_rest.get(..old_rest.len() - suffix).unwrap_or_default();
    let new_middle = new_rest.get(..new_rest.len() - suffix).unwrap_or_default();
    let runs = match matching_runs(old_middle, new_middle) {
        Some(matches) => gaps(&matches, old_middle.len(), new_middle.len()),
        None => vec![(0..old_middle.len(), 0..new_middle.len())],
    };
    runs.into_iter()
        .filter(|(old_run, new_run)| !old_run.is_empty() || !new_run.is_empty())
        .map(|(old_run, new_run)| {
            (
                old_run.start + prefix..old_run.end + prefix,
                new_run.start + prefix..new_run.end + prefix,
            )
        })
        .collect()
}

/// The runs between matching runs.
fn gaps(
    matches: &[(usize, usize, usize)],
    old_length: usize,
    new_length: usize,
) -> Vec<(Range<usize>, Range<usize>)> {
    let mut runs = Vec::new();
    let (mut old_at, mut new_at) = (0, 0);
    for &(old_start, new_start, length) in matches {
        if old_start > old_at || new_start > new_at {
            runs.push((old_at..old_start, new_at..new_start));
        }
        old_at = old_start + length;
        new_at = new_start + length;
    }
    if old_at < old_length || new_at < new_length {
        runs.push((old_at..old_length, new_at..new_length));
    }
    runs
}

/// Myers' shortest edit script between two lists of lines, as the runs of
/// lines they share: (start in old, start in new, length), in order.
/// `None` when it takes more than [`MOST_STEPS`] steps.
fn matching_runs(old: &[&str], new: &[&str]) -> Option<Vec<(usize, usize, usize)>> {
    let (n, m) = (signed(old.len())?, signed(new.len())?);
    let snake = |mut x: isize, mut y: isize| {
        while x < n && y < m && line(old, x) == line(new, y) && line(old, x).is_some() {
            x += 1;
            y += 1;
        }
        x
    };
    // `trace[d]` holds, for each diagonal k in -d..=d (at index k + d), how
    // far along the old lines the furthest path with d steps reaches.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = false;
    for d in 0..=MOST_STEPS {
        let steps = signed(d)?;
        let mut row = vec![0; 2 * d + 1];
        for k in (-steps..=steps).step_by(2) {
            let x = if d == 0 {
                0
            } else {
                let previous = trace.get(d - 1)?;
                let at = |k: isize| previous.get(usize::try_from(k + steps - 1).ok()?).copied();
                if k == -steps || (k != steps && at(k - 1)? < at(k + 1)?) {
                    at(k + 1)?
                } else {
                    at(k - 1)? + 1
                }
            };
            let x = snake(x, x - k);
            *row.get_mut(usize::try_from(k + steps).ok()?)? = x;
            if x >= n && x - k >= m {
                found = true;
            }
        }
        trace.push(row);
        if found {
            break;
        }
    }
    if !found {
        return None;
    }
    let mut matches = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (1..trace.len()).rev() {
        let steps = signed(d)?;
        let previous = trace.get(d - 1)?;
        let at = |k: isize| previous.get(usize::try_from(k + steps - 1).ok()?).copied();
        let k = x - y;
        let previous_k = if k == -steps || (k != steps && at(k - 1)? < at(k + 1)?) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = at(previous_k)?;
        let previous_y = previous_x - previous_k;
        let (middle_x, middle_y) = if previous_k == k + 1 {
            (previous_x, previous_y + 1)
        } else {
            (previous_x + 1, previous_y)
        };
        if x > middle_x {
            matches.push((
                usize::try_from(middle_x).ok()?,
                usize::try_from(middle_y).ok()?,
                usize::try_from(x - middle_x).ok()?,
            ));
        }
        (x, y) = (previous_x, previous_y);
    }
    if x > 0 {
        matches.push((0, 0, usize::try_from(x).ok()?));
    }
    matches.reverse();
    Some(matches)
}

fn signed(value: usize) -> Option<isize> {
    isize::try_from(value).ok()
}

fn line<'a>(lines: &[&'a str], index: isize) -> Option<&'a str> {
    lines.get(usize::try_from(index).ok()?).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(old: &str, new: &str) -> Vec<TextEdit> {
        let edits = text_edits(old, new);
        assert_eq!(apply_text_edits(old, &edits).as_deref(), Some(new));
        edits
    }

    #[test]
    fn a_changed_number_is_one_small_edit() {
        let edits = round_trip("a = 900mm\nb = 2\n", "a = 950mm\nb = 2\n");
        assert_eq!(
            edits,
            [TextEdit {
                range: 5..6,
                text: "5".to_owned()
            }]
        );
    }

    #[test]
    fn edits_come_last_in_the_file_first() {
        let edits = round_trip("one\ntwo\nthree\nfour\n", "zero\none\ntwo\nthree!\nfour\n");
        assert_eq!(edits.len(), 2);
        assert!(edits[0].range.start > edits[1].range.end);
    }

    #[test]
    fn round_trips_insertions_deletions_and_new_files() {
        round_trip("", "stave 1\n\npart a : s { }\n");
        round_trip("a\nb\nc\n", "");
        round_trip("a\nb\nc", "a\nc\nb\nd");
        round_trip("x\ny\n", "x\ny\n");
        round_trip("ça\nb\n", "çé\nb\nc\n");
        round_trip("a\nb\na\nb\na\n", "b\na\nb\nb\na\nc\n");
        assert!(text_edits("same\n", "same\n").is_empty());
    }

    #[test]
    fn edits_out_of_order_are_refused() {
        let edits = [
            TextEdit {
                range: 0..1,
                text: String::new(),
            },
            TextEdit {
                range: 2..3,
                text: String::new(),
            },
        ];
        assert_eq!(apply_text_edits("abc", &edits), None);
    }
}
