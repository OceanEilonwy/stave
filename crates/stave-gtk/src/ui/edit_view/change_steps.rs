//! Steps of an edit script (`--edit-script`, for `tools/window-shot`)
//! for the changes of the design, each doing what its gesture does
//! through the same window method:
//!
//! ```text
//! pick <name>                          a click on a part, piece of hardware or assembly: selected
//! repeat <count> [up|across|deep]      the card's count stepper (the candidate's, else the item's)
//! drag count <count>                   the count handle dragged until it says × count
//! drag boundary <n> <length>           the n-th boundary handle (from 1) dragged to a size
//! mirror                               "And the other side" switched on
//! hold <a> to <b>                      "Hold a distance…" from part a, then a click on b
//! hold min|max [<length>] "<reason>"   the hold card's fields
//! hold try                             its "Try it"
//! remove                               Delete on the selection
//! include dependants                   the removal's dialog: remove them too
//! rename <name>                        the item's name retyped in Details
//! finder <text>                        Ctrl+K with the text typed (the finder stays open)
//! do <text>                            Ctrl+K, the text, Enter: the first verb listed is run
//! ```

use stave_lang::placement::Axis;

use super::super::Window;
use super::verbs::{Run, parse_verb};

/// Reads `min|max [<length>] "<reason>"`.
fn hold_fields(rest: &str) -> Result<(bool, Option<String>, String), String> {
    let (head, reason) = rest
        .split_once('"')
        .ok_or("write it as hold min|max [<length>] \"<reason>\"")?;
    let reason = reason.trim_end().trim_end_matches('"').to_owned();
    let words: Vec<&str> = head.split_whitespace().collect();
    let (bound, value) = match words.as_slice() {
        [bound] => (*bound, None),
        [bound, value] => (*bound, Some((*value).to_owned())),
        _ => return Err("write it as hold min|max [<length>] \"<reason>\"".to_owned()),
    };
    let at_most = match bound {
        "min" => false,
        "max" => true,
        _ => return Err(format!("'{bound}' isn't min or max")),
    };
    Ok((at_most, value, reason))
}

impl Window {
    /// Does a step of an edit script that changes the design, or says the
    /// word isn't a step.
    pub(in crate::ui) fn change_step(&mut self, word: &str, rest: &str) -> Result<(), String> {
        // Each command is sent at once, so the next step sees it done.
        self.editing.immediate = true;
        let done = self.change_step_now(word, rest);
        self.editing.immediate = false;
        done
    }

    fn change_step_now(&mut self, word: &str, rest: &str) -> Result<(), String> {
        match word {
            "pick" => {
                let selection = {
                    let session = &self.session;
                    let (design, _) = session.drawable().ok_or("nothing is drawn yet")?;
                    stave_app::link::selection_named(design, rest)
                        .ok_or_else(|| format!("the design has nothing called '{rest}'"))?
                };
                self.cancel_edit();
                self.dispatch(stave_app::session::Command::Select(Some(selection)));
            }
            "repeat" if rest == "open" => {
                if !self.unfold_repeat() {
                    return Err("no folded \"Repeat…\" shows".to_owned());
                }
            }
            "repeat" => {
                // "Repeat…" first, as a person would, when it is folded.
                self.unfold_repeat();
                let words: Vec<&str> = rest.split_whitespace().collect();
                let (count, axis) = match words.as_slice() {
                    [count] => (*count, None),
                    [count, axis] => (
                        *count,
                        Some(
                            Axis::from_word(axis)
                                .ok_or_else(|| format!("'{axis}' isn't an axis"))?,
                        ),
                    ),
                    _ => return Err("write it as repeat <count> [up|across|deep]".to_owned()),
                };
                let count: usize = count
                    .parse()
                    .map_err(|_| format!("'{count}' isn't a count"))?;
                let axis = axis.or_else(|| Some(self.editing.repeats.shown.axis()));
                self.repeat_selected(count, axis);
            }
            "drag" => {
                let words: Vec<&str> = rest.split_whitespace().collect();
                match words.as_slice() {
                    ["count", count] => {
                        let count: usize = count
                            .parse()
                            .map_err(|_| format!("'{count}' isn't a count"))?;
                        self.drag_count_to(count)?;
                    }
                    ["boundary", index, length] => {
                        let index: usize = index
                            .parse()
                            .map_err(|_| format!("'{index}' isn't a number"))?;
                        let length =
                            stave_app::editing::parse_length(length).map_err(|error| error.0)?;
                        let boundaries = self.boundaries_shown();
                        let boundary = index
                            .checked_sub(1)
                            .and_then(|index| boundaries.get(index))
                            .ok_or_else(|| format!("{} boundary handles show", boundaries.len()))?
                            .clone();
                        self.move_boundary(&boundary, length);
                    }
                    _ => {
                        return Err(
                            "write it as drag count <count> or drag boundary <n> <length>"
                                .to_owned(),
                        );
                    }
                }
            }
            "mirror" => self.mirror_selected(),
            "hold" => {
                if let Some((first, second)) = rest.split_once(" to ") {
                    let first = stave_app::hold::Measured::Item(first.trim().to_owned());
                    let second = stave_app::hold::Measured::Item(second.trim().to_owned());
                    self.start_hold(first.clone());
                    self.ask_hold(first, second);
                } else if rest == "try" {
                    self.try_hold();
                } else {
                    let (at_most, value, reason) = hold_fields(rest)?;
                    self.fill_hold(at_most, value.as_deref(), &reason);
                }
            }
            "remove" => {
                if !self.remove_selected() {
                    return Err("nothing is selected that can be removed".to_owned());
                }
            }
            "include" => self.answer_dependants(true),
            "rename" => self.rename_selected(rest),
            "finder" => {
                self.widgets.finder.present(&self.widgets.window, rest);
            }
            "do" => {
                let verb = parse_verb(rest).ok_or_else(|| format!("'{rest}' names no verb"))?;
                let _ = self.finder_actions(rest);
                let run = self.editing.verbs.first().cloned();
                match run {
                    Some(run) => self.run_verb(run),
                    None => self.run_verb(Run::Other(verb)),
                }
            }
            _ => return Err(format!("'{word}' isn't a step of an edit script")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hold_cards_fields_are_read() {
        assert_eq!(
            hold_fields("min \"room for the drawers\""),
            Ok((false, None, "room for the drawers".to_owned()))
        );
        assert_eq!(
            hold_fields("max 400mm \"knee room\""),
            Ok((true, Some("400mm".to_owned()), "knee room".to_owned()))
        );
        assert!(hold_fields("least \"x\"").is_err());
        assert!(hold_fields("min").is_err());
    }
}
