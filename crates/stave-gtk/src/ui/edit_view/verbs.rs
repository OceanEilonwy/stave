//! The finder's verbs (Ctrl+K, `docs/gui.md`, "The finder's verbs"):
//! "add shelf" or "add drawer" with a space or a part selected starts the
//! same edit a click on its menu entry does; "remove", "rename <name>",
//! "repeat 3 up", "mirror" and "hold" do what the card's buttons do. Each
//! runs the same method of the window as the mouse: one path for both.

use stave_app::i18n::{fill_in, tr};

use stave_app::edit::{Declaration, Picked};
use stave_app::session::Selection;
use stave_lang::placement::Axis;
use stave_model::nonempty::NonEmpty;

use super::super::Window;
use super::super::finder::Action;
use super::item_tools::MOST;

/// A verb typed in the finder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Verb {
    /// "add shelf": choose the menu entry whose label holds the words
    /// (all of the menu when there are none).
    Add(String),
    /// "remove" or "delete".
    Remove,
    /// "rename door_left" (or "rename to door_left"); bare, the name field.
    Rename(Option<String>),
    /// "repeat", "repeat 3", "repeat 3 across".
    Repeat {
        /// How many.
        count: Option<usize>,
        /// Which way.
        axis: Option<Axis>,
    },
    /// "mirror" or "other side".
    Mirror,
    /// "hold".
    Hold,
}

/// The verb a query names, if it starts with one.
pub(in crate::ui) fn parse_verb(query: &str) -> Option<Verb> {
    let query = query.trim().to_lowercase();
    let mut words = query.split_whitespace();
    let first = words.next()?;
    let rest: Vec<&str> = words.collect();
    match first {
        "add" | "put" => Some(Verb::Add(rest.join(" "))),
        "remove" | "delete" => Some(Verb::Remove),
        "rename" => {
            let name = match rest.as_slice() {
                ["to", name] | [name] => Some((*name).to_owned()),
                _ => None,
            };
            Some(Verb::Rename(name))
        }
        "repeat" | "make" => {
            let mut count = None;
            let mut axis = None;
            for word in rest {
                if let Ok(number) = word.trim_start_matches('x').parse::<usize>() {
                    count = Some(number.clamp(1, MOST));
                } else if let Some(found) = axis_word(word) {
                    axis = Some(found);
                }
            }
            Some(Verb::Repeat { count, axis })
        }
        "mirror" => Some(Verb::Mirror),
        "other" if rest.first() == Some(&"side") => Some(Verb::Mirror),
        "hold" | "measure" => Some(Verb::Hold),
        _ => None,
    }
}

/// An axis as a user types it.
fn axis_word(word: &str) -> Option<Axis> {
    match word {
        "up" | "stacked" | "high" => Some(Axis::Up),
        "across" | "side" | "wide" => Some(Axis::Across),
        "deep" | "back" => Some(Axis::Deep),
        _ => None,
    }
}

/// `true` when a menu entry's label (or its group) holds the words.
pub(in crate::ui) fn label_matches(label: &str, group: &str, words: &str) -> bool {
    let words = words.trim().to_lowercase();
    if words.is_empty() {
        return true;
    }
    let label = label.to_lowercase();
    let spaced: String = label
        .chars()
        .flat_map(|c| [c].into_iter())
        .collect::<String>();
    label.contains(&words)
        || spaced.replace(' ', "").contains(&words.replace(' ', ""))
        || format!("{}.{label}", group.to_lowercase()) == words
}

/// What a verb run does, worked out when the finder lists it.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) enum Run {
    /// Choose this entry of the open menu.
    Choose(Declaration),
    /// Start an edit from this selection, then choose the first entry
    /// whose label holds the words.
    StartThenChoose(Picked, String),
    /// Any other verb, on the selection.
    Other(Verb),
}

impl Window {
    /// The verbs a query names, as the finder lists them; what each does
    /// is kept until one is chosen.
    pub(in crate::ui) fn finder_actions(&mut self, query: &str) -> Vec<Action> {
        let Some(verb) = parse_verb(query) else {
            self.editing.verbs = Vec::new();
            return Vec::new();
        };
        let session = &self.session;
        let item = self.selected_item();
        let mut found: Vec<(Action, Run)> = Vec::new();
        match &verb {
            Verb::Add(words) => {
                if let Some(edit) = session.edit().filter(|edit| edit.chosen().is_none()) {
                    let place = edit.picked().heading().text().to_lowercase();
                    for candidate in edit.menu() {
                        let group = super::super::edit_menu::group_title(&candidate.entry.group);
                        if label_matches(candidate.entry.label.as_str(), &group, words) {
                            found.push((
                                Action {
                                    label: fill_in(
                                        &tr("Add {name}"),
                                        &[("name", &candidate.entry.label.as_str())],
                                    ),
                                    detail: format!("{place} · {group}"),
                                },
                                Run::Choose(candidate.declaration),
                            ));
                        }
                    }
                } else if let Some(picked) = self.picked_for_verb(session) {
                    let label = if words.is_empty() {
                        tr("Add something")
                    } else {
                        fill_in(&tr("Add “{words}”"), &[("words", &words)])
                    };
                    let detail = match &item {
                        Some(item) => fill_in(&tr("onto {item}"), &[("item", item)]),
                        None => tr("onto the selection"),
                    };
                    found.push((
                        Action { label, detail },
                        Run::StartThenChoose(picked, words.clone()),
                    ));
                }
            }
            Verb::Remove => {
                if let Some(item) = &item {
                    found.push((
                        Action {
                            label: fill_in(&tr("Remove {item}…"), &[("item", item)]),
                            detail: tr("lists what goes with it first"),
                        },
                        Run::Other(verb.clone()),
                    ));
                }
            }
            Verb::Rename(to) => {
                if let Some(item) = &item {
                    found.push((
                        Action {
                            label: match to {
                                Some(to) => fill_in(
                                    &tr("Rename {item} to {to}"),
                                    &[("item", item), ("to", to)],
                                ),
                                None => fill_in(&tr("Rename {item}…"), &[("item", item)]),
                            },
                            detail: tr("everywhere it is used"),
                        },
                        Run::Other(verb.clone()),
                    ));
                }
            }
            Verb::Repeat { count, axis } => {
                let chosen = session
                    .edit()
                    .and_then(|edit| edit.chosen())
                    .map(|chosen| chosen.candidate().entry.label.as_str().to_owned());
                let what = chosen.or_else(|| item.clone());
                if let Some(what) = what {
                    let count = count.unwrap_or(2);
                    let label = match axis {
                        Some(axis) => fill_in(
                            &tr("Make {count} of {what} {way}"),
                            &[("count", &count), ("what", &what), ("way", &axis.word())],
                        ),
                        None => fill_in(
                            &tr("Make {count} of {what}"),
                            &[("count", &count), ("what", &what)],
                        ),
                    };
                    found.push((
                        Action {
                            label,
                            detail: tr("filling its space"),
                        },
                        Run::Other(verb.clone()),
                    ));
                }
            }
            Verb::Mirror => {
                if let Some(item) = &item {
                    found.push((
                        Action {
                            label: fill_in(&tr("And the other side of {item}"), &[("item", item)]),
                            detail: tr("adds its mirror image"),
                        },
                        Run::Other(verb.clone()),
                    ));
                }
            }
            Verb::Hold => {
                if matches!(session.selection(), Some(Selection::Part(_))) {
                    found.push((
                        Action {
                            label: tr("Hold a distance…"),
                            detail: match &item {
                                Some(item) => fill_in(&tr("from {item}"), &[("item", item)]),
                                None => tr("from the part"),
                            },
                        },
                        Run::Other(verb.clone()),
                    ));
                }
            }
        }
        if found.is_empty() {
            found.push((
                Action {
                    label: tr("Select something first"),
                    detail: tr("a part, or click in a space, then type the verb"),
                },
                Run::Other(Verb::Hold),
            ));
            self.editing.verbs = Vec::new();
            return found.into_iter().map(|(action, _)| action).collect();
        }
        let (actions, runs): (Vec<Action>, Vec<Run>) = found.into_iter().unzip();
        self.editing.verbs = runs;
        actions
    }

    /// The selection an "add" starts from when no menu is open: the face
    /// last clicked on the selected part, else the part itself.
    fn picked_for_verb(&self, session: &stave_app::session::Session) -> Option<Picked> {
        let Some(Selection::Part(part)) = session.selection() else {
            return None;
        };
        let face = self.editing.state.face;
        Some(match face {
            Some(face) if face.part == *part => Picked::Face(face),
            _ => Picked::Group(NonEmpty::new(vec![*part])?),
        })
    }

    /// Runs the verb the finder listed at `index`.
    pub(in crate::ui) fn run_finder_action(&mut self, index: usize) {
        let run = self.editing.verbs.get(index).cloned();
        if let Some(run) = run {
            self.run_verb(run);
        }
    }

    /// Runs a verb: the same methods the menu, the cards and the keys call.
    pub(in crate::ui) fn run_verb(&mut self, run: Run) {
        if !self.gestures_allowed() {
            self.edit_toast(&self.parse_block_line().unwrap_or_default());
            return;
        }
        match run {
            Run::Choose(declaration) => self.choose_later(declaration),
            Run::StartThenChoose(picked, words) => {
                let point = self.point_of(None);
                self.start_edit(picked, point);
                let declaration = self.session.edit().and_then(|edit| {
                    edit.menu()
                        .iter()
                        .find(|candidate| {
                            let group =
                                super::super::edit_menu::group_title(&candidate.entry.group);
                            label_matches(candidate.entry.label.as_str(), &group, &words)
                        })
                        .map(|candidate| candidate.declaration)
                });
                match declaration {
                    Some(declaration) if !words.is_empty() => self.choose_later(declaration),
                    Some(_) => {}
                    None => self.edit_toast(&fill_in(
                        &tr("Nothing called “{words}” can go there: the menu lists what can"),
                        &[("words", &words)],
                    )),
                }
            }
            Run::Other(Verb::Remove) => {
                self.remove_selected();
            }
            Run::Other(Verb::Rename(Some(to))) => self.rename_selected(&to),
            Run::Other(Verb::Rename(None)) => self.focus_item_name(),
            Run::Other(Verb::Repeat { count, axis }) => {
                self.repeat_selected(count.unwrap_or(2), axis.or(Some(Axis::Up)));
            }
            Run::Other(Verb::Mirror) => self.mirror_selected(),
            Run::Other(Verb::Hold) => self.hold_from_selected(),
            Run::Other(Verb::Add(words)) => {
                self.edit_toast(&fill_in(
                    &tr("Select a space or a part to add “{words}” to"),
                    &[("words", &words)],
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_query_names_a_verb() {
        assert_eq!(parse_verb("add shelf"), Some(Verb::Add("shelf".to_owned())));
        assert_eq!(
            parse_verb("  Add  Hanging Rail "),
            Some(Verb::Add("hanging rail".to_owned()))
        );
        assert_eq!(parse_verb("add"), Some(Verb::Add(String::new())));
        assert_eq!(parse_verb("delete"), Some(Verb::Remove));
        assert_eq!(parse_verb("remove"), Some(Verb::Remove));
        assert_eq!(
            parse_verb("rename to door_left"),
            Some(Verb::Rename(Some("door_left".to_owned())))
        );
        assert_eq!(parse_verb("rename"), Some(Verb::Rename(None)));
        assert_eq!(
            parse_verb("repeat 3 across"),
            Some(Verb::Repeat {
                count: Some(3),
                axis: Some(Axis::Across)
            })
        );
        assert_eq!(
            parse_verb("repeat x40"),
            Some(Verb::Repeat {
                count: Some(MOST),
                axis: None
            })
        );
        assert_eq!(parse_verb("other side"), Some(Verb::Mirror));
        assert_eq!(parse_verb("hold"), Some(Verb::Hold));
        assert_eq!(parse_verb("hinge"), None);
        assert_eq!(parse_verb(""), None);
    }

    #[test]
    fn an_entry_matches_the_words_typed() {
        assert!(label_matches("Shelf", "casework", "shelf"));
        assert!(label_matches("HangingRail", "casework", "hangingrail"));
        assert!(label_matches("HangingRail", "casework", "hanging rail"));
        assert!(label_matches("Drawer", "casework", "casework.drawer"));
        assert!(label_matches("Drawer", "casework", ""));
        assert!(!label_matches("Drawer", "casework", "shelf"));
    }
}
