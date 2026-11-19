//! The grip steps of an edit script (`--edit-script FILE`, for
//! `tools/window-shot`), each doing what the pointer or the keyboard
//! would on the grips, through the same functions:
//!
//! ```text
//! grips assembly <name>          the assembly selected: its grips on the model
//! grips envelope                 the envelope's grips (as a click in the envelope shows them)
//! grips split <piece>            the boundaries of a split (as a click in a piece shows them)
//! stops                          every grip's stops found now (the window finds them on a worker)
//! drag <grip> <value>            the handle dragged to a value and still held
//! release                        the handle let go: written when it fits
//! type <grip> <value>            the number clicked, the value typed and Enter pressed
//! pick <param>                   a size from the list a follows grip offers
//! menu <grip>                    the grip's menu with "Let the solver move this" opened
//! solver <grip> <low> <high>     the switch turned on with that range: the param made solved
//! plain <grip>                   the switch turned off: the param made plain again
//! ```
//!
//! Grips are numbered from 1 in the listing's order, as `stave edit`
//! numbers them.

use std::sync::atomic::AtomicBool;

use stave_app::change::ChangeDetail;
use stave_app::editing::{EditCommand, parse_value};
use stave_app::grips::GripSelection;
use stave_app::session::{Command, Selection};

use super::grip_text::{amount_like, grip_value, solver_switches};
use crate::ui::Window;

impl Window {
    /// Does a grip step of an edit script; `None` when `word` isn't one.
    pub(in crate::ui) fn grip_step(
        &mut self,
        word: &str,
        rest: &str,
    ) -> Option<Result<(), String>> {
        let result = match word {
            "grips" => self.grips_step(rest),
            "stops" => {
                let never = AtomicBool::new(false);
                let events = self.session.find_grip_stops(None, &never);
                self.handle(events);
                Ok(())
            }
            "drag" => self.numbered(rest).and_then(|(index, value)| {
                let listed = self.grip_at(index).ok_or("no such grip")?;
                let value = amount_like(grip_value(&listed), value)?;
                self.drag_grip_to(index, value);
                Ok(())
            }),
            "release" => {
                self.release_grip();
                Ok(())
            }
            "type" => self.numbered(rest).map(|(index, value)| {
                self.type_grip(index, value);
            }),
            "pick" => self.pick_step(rest),
            "menu" => self.numbered(&format!("{rest} -")).and_then(|(index, _)| {
                if self.open_grip_menu(index) {
                    Ok(())
                } else {
                    Err("that grip has no switch for the solver".to_owned())
                }
            }),
            "solver" => self.solver_step(rest, true),
            "plain" => self.solver_step(rest, false),
            _ => return None,
        };
        Some(result)
    }

    /// Whether a follows grip's list of sizes waits for a pick.
    pub(in crate::ui) fn grip_choice_waits(&self) -> bool {
        self.session
            .change()
            .is_some_and(|change| {
                matches!(change.detail(), ChangeDetail::Grip(detail) if !detail.choices.is_empty())
            })
    }

    fn grips_step(&mut self, rest: &str) -> Result<(), String> {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let selection = match words.as_slice() {
            ["envelope"] => GripSelection::Envelope,
            ["assembly", name] => {
                let id = {
                    let session = &self.session;
                    let (design, _) = session.drawable().ok_or("nothing is drawn yet")?;
                    design
                        .assemblies
                        .iter()
                        .find(|assembly| assembly.name == *name)
                        .map(|assembly| assembly.id)
                        .ok_or_else(|| format!("the design has no assembly called '{name}'"))?
                };
                // Selected as a click in the tree does (unless the command
                // line selected it already, keeping its camera).
                if self.session.selection() != Some(&Selection::Assembly(id)) {
                    self.dispatch(Command::Select(Some(Selection::Assembly(id))));
                }
                GripSelection::Assembly(id)
            }
            ["split", piece] => {
                let session = &self.session;
                let (_, graph) = session.drawable().ok_or("nothing is drawn yet")?;
                let region = graph
                    .regions()
                    .iter()
                    .find(|region| region.name == *piece)
                    .map(|region| region.id)
                    .ok_or_else(|| format!("the design has no space called '{piece}'"))?;
                GripSelection::Split(region)
            }
            _ => {
                return Err(
                    "write it as grips envelope, grips assembly <name> or grips split <piece>"
                        .to_owned(),
                );
            }
        };
        self.show_grips(Some(selection));
        if self.session.grips().is_none() {
            return Err(format!("'{rest}' has no grips"));
        }
        Ok(())
    }

    /// `<grip> <value>`: the grip's index from 1, and the rest.
    fn numbered<'a>(&self, rest: &'a str) -> Result<(usize, &'a str), String> {
        let (number, value) = rest
            .split_once(char::is_whitespace)
            .ok_or("write the grip's number, then the value")?;
        let index = number
            .parse::<usize>()
            .ok()
            .and_then(|n| n.checked_sub(1))
            .ok_or("grips count from 1")?;
        if self.grip_at(index).is_none() {
            return Err(format!("there is no grip {number}"));
        }
        Ok((index, value.trim()))
    }

    fn pick_step(&mut self, name: &str) -> Result<(), String> {
        let param = self
            .session
            .change()
            .and_then(|change| match change.detail() {
                ChangeDetail::Grip(detail) => detail
                    .choices
                    .iter()
                    .find(|choice| {
                        choice.name == name
                            || choice
                                .name
                                .rsplit('.')
                                .next()
                                .and_then(|t| t.split('[').next())
                                == Some(name)
                    })
                    .map(|choice| choice.param),
                _ => None,
            })
            .ok_or_else(|| format!("'{name}' is not one of the sizes to pick from"))?;
        self.dispatch(Command::Edit(EditCommand::PickParam(param)));
        Ok(())
    }

    fn solver_step(&mut self, rest: &str, solved: bool) -> Result<(), String> {
        let words: Vec<&str> = rest.split_whitespace().collect();
        let number = words.first().ok_or("write the grip's number")?;
        let (index, _) = self.numbered(&format!("{number} -"))?;
        let listed = self.grip_at(index).ok_or("no such grip")?;
        let switch = solver_switches(&listed)
            .into_iter()
            .next()
            .ok_or("that grip has no switch for the solver")?;
        let command = if solved {
            let [_, low, high] = words.as_slice() else {
                return Err("write it as solver <grip> <low> <high>".to_owned());
            };
            EditCommand::MakeSolved {
                param: switch.param,
                low: parse_value(low).map_err(|error| error.0)?,
                high: parse_value(high).map_err(|error| error.0)?,
            }
        } else {
            EditCommand::MakePlain(switch.param)
        };
        self.grip_change_opened(index);
        self.dispatch(Command::Edit(command));
        Ok(())
    }
}
