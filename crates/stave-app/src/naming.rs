//! The names the editor writes (`docs/editing.md`, "Naming rules"), as pure
//! functions: an assembly, joint or piece of hardware named from its
//! declaration, the labels of a repeat from the direction of its split, the
//! `_bay` region a repeat fills, the two sides of a mirrored pair and a
//! held rule named from the two things it measures.
//!
//! A name that is already taken in its scope, or reserved, moves to the
//! next number: `drawer`, then `drawer_2`, `drawer_3`. A keyword or a
//! built-in function's name is reserved, so it starts at `_2`. What is
//! taken is a [`TakenNames`]: the caller fills it from the project (see
//! [`TakenNames::of_project`]), so these functions never look at a design.
//!
//! The tool never invents a param name: the envelope's three params are
//! fixed ([`ENVELOPE_PARAMS`], [`ENVELOPE_REGION`]) and every other param
//! is named by the user.

use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use stave_lang::ast::{Ident, Span};
use stave_lang::placement::Axis;
use stave_lang::{BUILTIN_FUNCTIONS, is_keyword};
use stave_model::Model;
use stave_model::defs::Scope;

/// The three params a project started from a box has, across, up and deep:
/// the only params the tool names by itself.
pub const ENVELOPE_PARAMS: [&str; 3] = ["overall_width", "overall_height", "overall_depth"];

/// The root region of a project started from a box.
pub const ENVELOPE_REGION: &str = "envelope";

/// Why a name can't be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NamingError {
    /// What the name is made from has no letters a name can use.
    #[error("'{0}' gives no name: a name needs a letter")]
    NoName(String),
    /// Every number up to the limit is taken (a bug, or a design with
    /// billions of names).
    #[error("every name from '{0}' is taken")]
    Exhausted(String),
}

/// The names already used in a scope, which a generated name must not
/// take.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TakenNames {
    names: BTreeSet<String>,
}

impl TakenNames {
    /// No names taken.
    pub fn new() -> Self {
        Self::default()
    }

    /// The names a new item of the project's module can't take: every
    /// item declared at the top level of the project's module (an array's
    /// name included once) and every name a file of the project imports a
    /// module under.
    pub fn of_project(model: &Model) -> Self {
        let defs = &model.defs;
        let mut names = BTreeSet::new();
        if let Some(module) = defs.modules.get(model.root.index()) {
            for item in &module.items {
                if let Some(decl) = defs.decl(*item)
                    && decl.scope == Scope::Module(model.root)
                {
                    names.insert(decl.name.clone());
                }
            }
        }
        for scope in defs.files.values() {
            if scope.module == Some(model.root) {
                names.extend(scope.imports.keys().cloned());
            }
        }
        Self { names }
    }

    /// Marks a name as taken, as when an earlier operation of the same
    /// patch adds it.
    pub fn insert(&mut self, name: impl Into<String>) {
        self.names.insert(name.into());
    }

    /// Frees a name, as for the item a change renames (its own name is
    /// not in the way of its new one).
    pub fn remove(&mut self, name: &str) {
        self.names.remove(name);
    }

    /// `true` if the name is taken.
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// `true` if a generated name can't be this word: it is taken, or it is
    /// a keyword or a built-in function's name.
    pub fn is_unavailable(&self, name: &str) -> bool {
        self.contains(name) || is_reserved(name)
    }
}

impl<S: Into<String>> FromIterator<S> for TakenNames {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        Self {
            names: iter.into_iter().map(Into::into).collect(),
        }
    }
}

/// `true` for a keyword or a built-in function's name: a name the tool
/// never writes bare.
pub fn is_reserved(name: &str) -> bool {
    is_keyword(name) || BUILTIN_FUNCTIONS.contains(&name)
}

/// A declaration's name as an item name: lower case, words joined by `_`.
/// `Drawer` gives `drawer`, `FramePanelDoor` gives `frame_panel_door`,
/// `dado` stays `dado`, and a run of capitals is one word (`MDFPanel` gives
/// `mdf_panel`).
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (index, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() {
            let previous = index.checked_sub(1).and_then(|i| chars.get(i));
            let next = chars.get(index + 1);
            let starts_word = match previous {
                None => false,
                Some(p) if *p == '_' => false,
                Some(p) if p.is_ascii_lowercase() || p.is_ascii_digit() => true,
                Some(p) if p.is_ascii_uppercase() => next.is_some_and(char::is_ascii_lowercase),
                Some(_) => false,
            };
            if starts_word {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    let collapsed = out
        .split('_')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    match collapsed.chars().next() {
        Some(first) if first.is_ascii_digit() => format!("n_{collapsed}"),
        _ => collapsed,
    }
}

/// The first of `base`, `base_2`, `base_3`, ... that is free in `taken`. A
/// reserved `base` starts at `base_2`.
///
/// # Errors
///
/// [`NamingError::NoName`] when `base` isn't a word.
pub fn first_free(base: &str, taken: &TakenNames) -> Result<Ident, NamingError> {
    numbered(base, taken, |candidate| vec![candidate.to_owned()])
        .map(|mut names| names.swap_remove(0))
}

/// The first number `n` (1 meaning no suffix) for which every name
/// `make(base_n)` gives is free, and those names.
fn numbered(
    base: &str,
    taken: &TakenNames,
    make: impl Fn(&str) -> Vec<String>,
) -> Result<Vec<Ident>, NamingError> {
    let no_name = || NamingError::NoName(base.to_owned());
    if !stave_lang::ast::is_word(base) {
        return Err(no_name());
    }
    for number in 1..=u32::MAX {
        let stem = if number == 1 {
            base.to_owned()
        } else {
            format!("{base}_{number}")
        };
        let names = make(&stem);
        if names.iter().all(|name| !taken.is_unavailable(name)) {
            return names
                .into_iter()
                .map(|name| Ident::new(name, Span::default()).map_err(|_| no_name()))
                .collect();
        }
    }
    Err(NamingError::Exhausted(base.to_owned()))
}

/// The name of a new assembly of a component: the component's
/// [`short_name`], numbered when taken (`drawer`, `drawer_2`;
/// `FramePanelDoor` gives `door`).
///
/// # Errors
///
/// [`NamingError::NoName`] when the component's name has no letters.
pub fn assembly_name(component: &str, taken: &TakenNames) -> Result<Ident, NamingError> {
    first_free(&short_name(component)?, taken)
}

/// The base name a component gives its assemblies (and the regions a
/// repeat of one fills): its name in [`snake_case`] when that is one
/// word (`Drawer` gives `drawer`), otherwise its last word, the thing it
/// is (`FramePanelDoor` gives `door`, `HangingRail` gives `rail`). A last
/// word that is a number or a single letter is kept with the word before
/// it (`WidePanel_2` gives `panel_2`, `PanelA` gives `panel_a`).
///
/// # Errors
///
/// [`NamingError::NoName`] when the component's name has no letters.
pub fn short_name(component: &str) -> Result<String, NamingError> {
    let full = base_of(component)?;
    let words: Vec<&str> = full.split('_').collect();
    let Some((last, before)) = words.split_last() else {
        return Ok(full);
    };
    let weak = last.len() < 2 || last.chars().all(|c| c.is_ascii_digit());
    let short = match (before.last(), weak) {
        (Some(previous), true) => format!("{previous}_{last}"),
        _ => (*last).to_owned(),
    };
    if stave_lang::ast::is_word(&short) && short.starts_with(|c: char| c.is_ascii_lowercase()) {
        Ok(short)
    } else {
        Ok(full)
    }
}

/// The name of a new joint or piece of hardware: its kind's name, numbered
/// (`dado_2`, `dado_3`). The kind's own name is never free: a joint named
/// `dado` would hide the kind `dado` from its own declaration
/// (`'dado' here is this joint itself, not a kind`), so the first takes
/// `_2`.
///
/// # Errors
///
/// [`NamingError::NoName`] when the kind's name has no letters.
pub fn kind_item_name(kind: &str, taken: &TakenNames) -> Result<Ident, NamingError> {
    let base = base_of(kind)?;
    let mut taken = taken.clone();
    taken.insert(kind);
    taken.insert(base.as_str());
    first_free(&base, &taken)
}

/// The region a repeated component fills: the component's [`short_name`]
/// plus `_bay`, numbered when taken (`drawer_bay`, `drawer_bay_2`,
/// `door_bay`).
///
/// # Errors
///
/// [`NamingError::NoName`] when the component's name has no letters.
pub fn bay_name(component: &str, taken: &TakenNames) -> Result<Ident, NamingError> {
    first_free(&format!("{}_bay", short_name(component)?), taken)
}

fn base_of(name: &str) -> Result<String, NamingError> {
    let base = snake_case(name);
    if base.is_empty() {
        Err(NamingError::NoName(name.to_owned()))
    } else {
        Ok(base)
    }
}

/// How many pieces a repeat makes: two or more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RepeatCount(NonZeroUsize);

impl RepeatCount {
    /// The count, or `None` for fewer than two pieces.
    pub fn new(count: usize) -> Option<Self> {
        if count >= 2 {
            NonZeroUsize::new(count).map(Self)
        } else {
            None
        }
    }

    /// The number of pieces.
    pub fn get(self) -> usize {
        self.0.get()
    }
}

/// How the array of a repeat is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepeatArray {
    /// Two or three pieces: labels from the direction of the split, in
    /// label order (`[bottom, top]`).
    Labels(Vec<&'static str>),
    /// Four or more: a counted array (`[4]`).
    Count(RepeatCount),
}

impl RepeatArray {
    /// The array as declared after a name: `[bottom, top]`, `[4]`.
    pub fn declaration(&self) -> String {
        match self {
            RepeatArray::Labels(labels) => format!("[{}]", labels.join(", ")),
            RepeatArray::Count(count) => format!("[{}]", count.get()),
        }
    }

    /// The index of each piece, in order: its label, or its number from 0.
    pub fn indexes(&self) -> Vec<String> {
        match self {
            RepeatArray::Labels(labels) => labels.iter().map(|label| (*label).to_owned()).collect(),
            RepeatArray::Count(count) => (0..count.get()).map(|n| n.to_string()).collect(),
        }
    }
}

/// The array of a repeat along `axis`: up `[bottom, top]` or `[bottom,
/// middle, top]`, across `[left, right]` or `[left, middle, right]`, deep
/// `[front, back]` or `[front, middle, back]`; four or more pieces are
/// counted. Labels run in the direction `split` labels its pieces.
pub fn repeat_array(axis: Axis, count: RepeatCount) -> RepeatArray {
    let (first, last) = sides(axis);
    match count.get() {
        2 => RepeatArray::Labels(vec![first, last]),
        3 => RepeatArray::Labels(vec![first, "middle", last]),
        _ => RepeatArray::Count(count),
    }
}

/// The two ends of an axis, in the order `split` labels them.
fn sides(axis: Axis) -> (&'static str, &'static str) {
    match axis {
        Axis::Up => ("bottom", "top"),
        Axis::Across => ("left", "right"),
        Axis::Deep => ("front", "back"),
    }
}

/// The two names of a mirrored pair across `axis`: the name plus each side
/// (`door_left`, `door_right` across; `_bottom`, `_top` up; `_front`,
/// `_back` deep). The first is the side nearer the start of the axis. A
/// name that already ends in a side of the mirrored axis has it replaced,
/// never doubled: `door_left` across gives `door_left` and `door_right`.
/// A side of another axis is kept as part of the name: `leg_front`
/// across gives `leg_front_left` and `leg_front_right`. When either
/// is taken, both move to the next number together (`door_2_left`,
/// `door_2_right`), so the pair still reads as a pair.
///
/// # Errors
///
/// [`NamingError::NoName`] when `base` has no letters.
pub fn mirrored_pair(
    base: &str,
    axis: Axis,
    taken: &TakenNames,
) -> Result<(Ident, Ident), NamingError> {
    let (first, last) = sides(axis);
    let names = numbered(&without_side(&base_of(base)?, axis), taken, |stem| {
        vec![format!("{stem}_{first}"), format!("{stem}_{last}")]
    })?;
    let mut names = names.into_iter();
    match (names.next(), names.next()) {
        (Some(a), Some(b)) => Ok((a, b)),
        _ => Err(NamingError::NoName(base.to_owned())),
    }
}

/// A name with a trailing side word of `axis` (`_left` or `_right`
/// across, `_bottom` or `_top` up, `_front` or `_back` deep) taken off,
/// when something is left. Side words of the other axes stay.
fn without_side(name: &str, axis: Axis) -> String {
    let (first, last) = sides(axis);
    for side in [first, last] {
        if let Some(stem) = name.strip_suffix(side).and_then(|s| s.strip_suffix('_'))
            && !stem.is_empty()
        {
            return stem.to_owned();
        }
    }
    name.to_owned()
}

/// Which way a held rule pins a distance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bound {
    /// At least the distance pinned: `>=`, named `_min`.
    Min,
    /// At most: `<=`, named `_max`.
    Max,
}

impl Bound {
    /// The suffix of the rule's name.
    pub fn suffix(self) -> &'static str {
        match self {
            Bound::Min => "min",
            Bound::Max => "max",
        }
    }

    /// The comparison the rule writes.
    pub fn operator(self) -> &'static str {
        match self {
            Bound::Min => ">=",
            Bound::Max => "<=",
        }
    }
}

/// The name of a held rule: the two things measured, joined by `_to_`,
/// plus `_min` or `_max` (`between(slat[0], apron_front) >= ...` gives
/// `slat_to_apron_front_min`). Each thing is named by its path with indexes
/// left out and members joined by `_` (`drawer[top].front` gives
/// `drawer_front`), and a word already in the name is not written again
/// (`rail.rail` to `divider.panel` gives `rail_to_divider_panel_min`, and
/// `drawer.front` to `drawer.back` gives `drawer_front_to_back_min`).
///
/// # Errors
///
/// [`NamingError::NoName`] when either thing has no letters.
pub fn held_rule_name(
    first: &str,
    second: &str,
    bound: Bound,
    taken: &TakenNames,
) -> Result<Ident, NamingError> {
    let a = path_words(first)?;
    let b = path_words(second)?;
    let mut seen: Vec<String> = Vec::new();
    let mut sides: [Vec<String>; 2] = [Vec::new(), Vec::new()];
    for (side, words) in sides.iter_mut().zip([&a, &b]) {
        for word in words.split('_') {
            if !seen.iter().any(|w| w == word) {
                seen.push(word.to_owned());
                side.push(word.to_owned());
            }
        }
    }
    let [left, mut right] = sides;
    if right.is_empty() {
        // The second is wholly in the first's words: keep its last word.
        right.push(b.rsplit('_').next().unwrap_or(&b).to_owned());
    }
    first_free(
        &format!(
            "{}_to_{}_{}",
            left.join("_"),
            right.join("_"),
            bound.suffix()
        ),
        taken,
    )
}

/// A path as words: indexes dropped, members joined by `_`.
fn path_words(path: &str) -> Result<String, NamingError> {
    let mut words = String::new();
    let mut depth = 0usize;
    for c in path.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            '.' => words.push('_'),
            c => words.push(c),
        }
    }
    base_of(&words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taken(names: &[&str]) -> TakenNames {
        names.iter().copied().collect()
    }

    #[test]
    fn declaration_names_become_item_names() {
        assert_eq!(snake_case("Drawer"), "drawer");
        assert_eq!(snake_case("FramePanelDoor"), "frame_panel_door");
        assert_eq!(snake_case("dado"), "dado");
        assert_eq!(snake_case("side_mount_slide"), "side_mount_slide");
        assert_eq!(snake_case("MDFPanel"), "mdf_panel");
        assert_eq!(snake_case("Shelf2Wide"), "shelf2_wide");
    }

    #[test]
    fn assemblies_take_the_next_number() {
        let none = TakenNames::new();
        assert_eq!(assembly_name("Drawer", &none).unwrap().as_str(), "drawer");
        // A name of two or more words is shortened to its last word.
        assert_eq!(
            assembly_name("FramePanelDoor", &none).unwrap().as_str(),
            "door"
        );
        assert_eq!(
            assembly_name("HangingRail", &none).unwrap().as_str(),
            "rail"
        );
        assert_eq!(short_name("Shelf2Wide").unwrap(), "wide");
        assert_eq!(short_name("PanelA").unwrap(), "panel_a");
        assert_eq!(short_name("Panel2").unwrap(), "panel2");
        assert_eq!(short_name("WidePanel_2").unwrap(), "panel_2");
        let some = taken(&["drawer", "drawer_2"]);
        assert_eq!(assembly_name("Drawer", &some).unwrap().as_str(), "drawer_3");
        // A joint never takes its kind's own name, which it would hide.
        assert_eq!(kind_item_name("dado", &none).unwrap().as_str(), "dado_2");
        assert_eq!(
            kind_item_name("dado", &taken(&["dado_2"]))
                .unwrap()
                .as_str(),
            "dado_3"
        );
        assert_eq!(
            kind_item_name("bar_pull", &none).unwrap().as_str(),
            "bar_pull_2"
        );
    }

    #[test]
    fn reserved_words_start_at_two() {
        let none = TakenNames::new();
        // `split` is a built-in function, `part` a keyword.
        assert_eq!(first_free("split", &none).unwrap().as_str(), "split_2");
        assert_eq!(assembly_name("Part", &none).unwrap().as_str(), "part_2");
        assert!(none.is_unavailable("box"));
        assert!(!none.is_unavailable("drawer"));
        assert!(matches!(
            assembly_name("!!", &none),
            Err(NamingError::NoName(_))
        ));
    }

    #[test]
    fn repeats_are_labelled_from_the_split_direction() {
        let count = |n| RepeatCount::new(n).unwrap();
        assert_eq!(
            repeat_array(Axis::Up, count(2)),
            RepeatArray::Labels(vec!["bottom", "top"])
        );
        assert_eq!(
            repeat_array(Axis::Up, count(3)),
            RepeatArray::Labels(vec!["bottom", "middle", "top"])
        );
        assert_eq!(
            repeat_array(Axis::Across, count(2)).declaration(),
            "[left, right]"
        );
        assert_eq!(
            repeat_array(Axis::Across, count(3)).declaration(),
            "[left, middle, right]"
        );
        assert_eq!(
            repeat_array(Axis::Deep, count(2)).declaration(),
            "[front, back]"
        );
        assert_eq!(
            repeat_array(Axis::Deep, count(3)).declaration(),
            "[front, middle, back]"
        );
        let four = repeat_array(Axis::Up, count(4));
        assert_eq!(four, RepeatArray::Count(count(4)));
        assert_eq!(four.declaration(), "[4]");
        assert_eq!(four.indexes(), ["0", "1", "2", "3"]);
        assert_eq!(RepeatCount::new(1), None);
        assert_eq!(RepeatCount::new(0), None);
    }

    #[test]
    fn bays_are_named_after_the_component() {
        let none = TakenNames::new();
        assert_eq!(bay_name("Drawer", &none).unwrap().as_str(), "drawer_bay");
        assert_eq!(
            bay_name("FramePanelDoor", &none).unwrap().as_str(),
            "door_bay"
        );
        assert_eq!(
            bay_name("Drawer", &taken(&["drawer_bay"]))
                .unwrap()
                .as_str(),
            "drawer_bay_2"
        );
    }

    #[test]
    fn mirrored_pairs_take_their_sides_and_move_together() {
        let none = TakenNames::new();
        let (left, right) = mirrored_pair("door", Axis::Across, &none).unwrap();
        assert_eq!((left.as_str(), right.as_str()), ("door_left", "door_right"));
        let (bottom, top) = mirrored_pair("Drawer", Axis::Up, &none).unwrap();
        assert_eq!(
            (bottom.as_str(), top.as_str()),
            ("drawer_bottom", "drawer_top")
        );
        // One side taken: both move on, so the pair keeps one stem.
        let (left, right) = mirrored_pair("door", Axis::Across, &taken(&["door_right"])).unwrap();
        assert_eq!(
            (left.as_str(), right.as_str()),
            ("door_2_left", "door_2_right")
        );
        // A name that has a side already has it replaced, not doubled.
        let (left, right) = mirrored_pair("door_left", Axis::Across, &none).unwrap();
        assert_eq!((left.as_str(), right.as_str()), ("door_left", "door_right"));
        let (front, back) = mirrored_pair("apron_front", Axis::Deep, &none).unwrap();
        assert_eq!(
            (front.as_str(), back.as_str()),
            ("apron_front", "apron_back")
        );
        let (left, right) = mirrored_pair("left", Axis::Across, &none).unwrap();
        assert_eq!((left.as_str(), right.as_str()), ("left_left", "left_right"));
        // A side of another axis is part of the name and stays.
        let (left, right) = mirrored_pair("leg_front", Axis::Across, &none).unwrap();
        assert_eq!(
            (left.as_str(), right.as_str()),
            ("leg_front_left", "leg_front_right")
        );
        let (left, right) = mirrored_pair("rail_back", Axis::Across, &none).unwrap();
        assert_eq!(
            (left.as_str(), right.as_str()),
            ("rail_back_left", "rail_back_right")
        );
        let (front, back) = mirrored_pair("drawer_top", Axis::Deep, &none).unwrap();
        assert_eq!(
            (front.as_str(), back.as_str()),
            ("drawer_top_front", "drawer_top_back")
        );
        let (bottom, top) = mirrored_pair("door_left", Axis::Up, &none).unwrap();
        assert_eq!(
            (bottom.as_str(), top.as_str()),
            ("door_left_bottom", "door_left_top")
        );
    }

    #[test]
    fn held_rules_are_named_from_what_they_measure() {
        let none = TakenNames::new();
        assert_eq!(
            held_rule_name("slat[0]", "apron_front", Bound::Min, &none)
                .unwrap()
                .as_str(),
            "slat_to_apron_front_min"
        );
        assert_eq!(
            held_rule_name("drawer[top].front", "rail", Bound::Max, &none)
                .unwrap()
                .as_str(),
            "drawer_front_to_rail_max"
        );
        assert_eq!(
            held_rule_name(
                "slat[0]",
                "apron_front",
                Bound::Min,
                &taken(&["slat_to_apron_front_min"])
            )
            .unwrap()
            .as_str(),
            "slat_to_apron_front_min_2"
        );
        // A word is never written twice.
        assert_eq!(
            held_rule_name("hanging_rail.rail", "divider.panel", Bound::Min, &none)
                .unwrap()
                .as_str(),
            "hanging_rail_to_divider_panel_min"
        );
        assert_eq!(
            held_rule_name("drawer.front", "drawer.back", Bound::Max, &none)
                .unwrap()
                .as_str(),
            "drawer_front_to_back_max"
        );
        assert_eq!(
            held_rule_name("rail", "rail.end", Bound::Min, &none)
                .unwrap()
                .as_str(),
            "rail_to_end_min"
        );
        assert_eq!(Bound::Min.operator(), ">=");
        assert_eq!(Bound::Max.operator(), "<=");
    }

    #[test]
    fn the_envelope_names_are_fixed_and_not_reserved_words() {
        let none = TakenNames::new();
        for name in ENVELOPE_PARAMS.iter().chain([&ENVELOPE_REGION]) {
            assert!(!none.is_unavailable(name), "{name}");
        }
    }
}
