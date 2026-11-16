//! "Did you mean ...?" suggestions for unknown names and fields.

/// The number of single-character insertions, deletions, substitutions
/// and swaps of two neighbouring characters that turn `a` into `b`
/// (optimal string alignment distance), on characters.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let width = b.len() + 1;
    let mut table = vec![0usize; (a.len() + 1) * width];
    let at = |i: usize, j: usize| i * width + j;
    for i in 0..=a.len() {
        if let Some(cell) = table.get_mut(at(i, 0)) {
            *cell = i;
        }
    }
    for j in 0..=b.len() {
        if let Some(cell) = table.get_mut(at(0, j)) {
            *cell = j;
        }
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let get = |table: &[usize], i: usize, j: usize| {
                table.get(at(i, j)).copied().unwrap_or(usize::MAX)
            };
            let different = usize::from(a.get(i - 1) != b.get(j - 1));
            let mut best = get(&table, i - 1, j)
                .saturating_add(1)
                .min(get(&table, i, j - 1).saturating_add(1))
                .min(get(&table, i - 1, j - 1).saturating_add(different));
            if i > 1 && j > 1 && a.get(i - 1) == b.get(j - 2) && a.get(i - 2) == b.get(j - 1) {
                best = best.min(get(&table, i - 2, j - 2).saturating_add(1));
            }
            if let Some(cell) = table.get_mut(at(i, j)) {
                *cell = best;
            }
        }
    }
    table.get(at(a.len(), b.len())).copied().unwrap_or(0)
}

/// The candidates close enough to `word` to be worth suggesting, nearest
/// first (ties in alphabetical order), at most two. A candidate is close
/// when at most a third of the word's characters (at least one) differ,
/// and fewer than all of them.
pub fn closest<S: AsRef<str>>(word: &str, candidates: impl IntoIterator<Item = S>) -> Vec<String> {
    let length = word.chars().count();
    let limit = (length / 3).max(1);
    let mut scored: Vec<(usize, String)> = candidates
        .into_iter()
        .map(|c| c.as_ref().to_owned())
        .filter(|c| c != word)
        .map(|c| (edit_distance(&word.to_lowercase(), &c.to_lowercase()), c))
        .filter(|(distance, _)| *distance <= limit && *distance < length)
        .collect();
    scored.sort();
    scored.dedup_by(|a, b| a.1 == b.1);
    scored.into_iter().take(2).map(|(_, c)| c).collect()
}

/// `did you mean 'top'?` or `did you mean 'top' or 'tip'?`, when a
/// candidate is close enough.
pub fn did_you_mean<S: AsRef<str>>(
    word: &str,
    candidates: impl IntoIterator<Item = S>,
) -> Option<String> {
    match closest(word, candidates).as_slice() {
        [] => None,
        [one] => Some(format!("did you mean '{one}'?")),
        [one, two, ..] => Some(format!("did you mean '{one}' or '{two}'?")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_count_edits() {
        assert_eq!(edit_distance("pine_18", "pine_19"), 1);
        assert_eq!(edit_distance("flsh", "flush"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("side", "side"), 0);
        assert_eq!(edit_distance("widht", "width"), 1);
    }

    #[test]
    fn suggestions_are_close_and_few() {
        assert_eq!(
            did_you_mean("pine_18", ["pine_19", "oak_19", "pine_12"]),
            Some("did you mean 'pine_12' or 'pine_19'?".into())
        );
        assert_eq!(
            did_you_mean("tp", ["top", "back"]),
            Some("did you mean 'top'?".into())
        );
        assert_eq!(did_you_mean("xyz", ["top", "back"]), None);
        assert_eq!(did_you_mean("a", ["b"]), None);
    }
}
