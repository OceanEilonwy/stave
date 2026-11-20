//! The words that are never items, from the language's own catalog
//! (`stave-schema`), so the editor and the checker share one list: enum
//! words (namespace 4), built-in functions, and the call heads fixed by
//! position (namespace 3).

use stave_schema::functions::Builtin;
use stave_schema::operations::Selector;
use stave_schema::placement::{AssemblyStepKind, MadePartTerm, PlacementTerm, SpacingKind};
use stave_schema::recipes::{CuttingStep, MarkingStep, NeedKind};
use stave_schema::words::WordSet;
use stave_schema::{CutOperation, Operation};

/// Fields whose value is a word of a closed list: `grain: length`,
/// `units: mm`, `form: plywood`, `species: oak`, `measure: diameter`.
pub(crate) const WORD_FIELDS: [&str; 5] = ["grain", "units", "form", "species", "measure"];

/// `true` if `word` is in one of the closed word lists.
pub fn is_enum_word(word: &str) -> bool {
    WordSet::ALL.iter().any(|set| set.contains(word))
}

/// `true` if `word` names a feature a recipe step works on (`waste`,
/// `cheeks`, `floor`). Unlike [`is_enum_word`], a feature word may also
/// be an item's name (a part called `floor`), so it never stops a name
/// from resolving or being chosen in a rename: it only colours a word
/// that resolves to nothing.
pub fn is_feature_word(word: &str) -> bool {
    stave_schema::operations::Feature::from_word(word).is_some()
}

/// `true` if `word` is the head of a call whose meaning is fixed by its
/// position: a placement term, a spacing, a made part's term, an assembly
/// step, a cut selector or operation, a recipe step or need, an edge
/// profile's `edge` or `around`, a hardware kind's `point`.
pub fn is_fixed_call(word: &str) -> bool {
    PlacementTerm::from_word(word).is_some()
        || SpacingKind::from_word(word).is_some()
        || MadePartTerm::from_word(word).is_some()
        || AssemblyStepKind::from_word(word).is_some()
        || Selector::from_word(word).is_some()
        || CutOperation::from_word(word).is_some()
        || Operation::from_word(word).is_some()
        || MarkingStep::from_word(word).is_some()
        || CuttingStep::from_word(word).is_some()
        || NeedKind::from_word(word).is_some()
        || word == stave_schema::placement::EXTEND
        || word == stave_schema::operations::MARK
        || word == stave_schema::operations::EDGE
        || word == stave_schema::operations::AROUND
        || word == stave_schema::functions::POINT
}

/// A built-in function's signature: `overlay(left: Length, ...) -> Mount`.
pub fn builtin_signature(word: &str) -> Option<String> {
    let builtin = Builtin::from_word(word)?;
    let parameters: Vec<String> = builtin
        .parameters()
        .iter()
        .map(|parameter| {
            if parameter.positional {
                parameter.name.to_owned()
            } else if parameter.required {
                format!("{}:", parameter.name)
            } else {
                format!("{}:?", parameter.name)
            }
        })
        .collect();
    Some(format!(
        "{}({}) → {}",
        builtin.word(),
        parameters.join(", "),
        builtin.returns().word()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_come_from_the_catalog() {
        assert!(is_enum_word("across"));
        assert!(is_enum_word("plywood"));
        assert!(!is_enum_word("door_l"));
        assert!(is_feature_word("cheeks"));
        assert!(!is_enum_word("floor"), "a part may be called floor");
        assert!(is_fixed_call("flush"));
        assert!(is_fixed_call("tenon"));
        assert!(!is_fixed_call("overlay"));
        assert!(builtin_signature("overlay").is_some_and(|s| s.starts_with("overlay(left:?")));
        assert_eq!(builtin_signature("door_l"), None);
    }
}
