//! A state of editing shown without a pointer (`--edit-script FILE`, for
//! `tools/window-shot`): each line is a step that does what a gesture
//! would, through the same session commands, once the design is drawn.
//!
//! ```text
//! hover <region>                       the pointer rests in a space: lit
//! select region <name>                 a click in a space: its menu, checked
//! select face <part>.<face>            the part's "Add onto" button for that face
//! select between <part> <part>         a click in the gap between two parts
//! click face <part>.<face>             a click on a part: selected, its card offering that face
//! choose <entry>                       a menu entry, by label or group.label
//! bind region|face|between ...         the next click, for the card's place
//! set <name> = <value>                 a setting typed on the card
//! name <name>                          the name retyped on the card
//! fix <problem> <fix>                  a fix's button, both counted from 1
//! add | cancel                         the card's buttons
//! ```
//!
//! `#` starts a comment. A step that can't be done is said on standard
//! error with its line, and the steps after it are skipped.

use std::rc::Rc;

use stave_app::edit::{FacingPair, PartFace, Picked};
use stave_app::editing::{EditCommand, entry_matches, parse_name, parse_value};
use stave_lang::placement::Face;
use stave_model::solved::{PartId, RegionId};
use stave_scene::{Aabb, Item};

use super::Window;

impl Window {
    /// Runs the edit script the command line gave, once. `true` when it
    /// ran (the window draws again before any screenshot is taken).
    pub(super) fn run_edit_script(self: &Rc<Self>) -> bool {
        let Some(path) = self.state.borrow_mut().edit_script.take() else {
            return false;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("error: can't read '{}': {error}", path.display());
                return true;
            }
        };
        for (number, line) in text.lines().enumerate() {
            let step = line.split('#').next().unwrap_or_default().trim();
            if step.is_empty() {
                continue;
            }
            if let Err(error) = self.edit_step(step) {
                eprintln!(
                    "error: {}:{}: {error}",
                    path.display(),
                    number.saturating_add(1)
                );
                break;
            }
        }
        true
    }

    /// Does one step of an edit script.
    fn edit_step(self: &Rc<Self>, step: &str) -> Result<(), String> {
        let (word, rest) = step
            .split_once(char::is_whitespace)
            .map_or((step, ""), |(word, rest)| (word, rest.trim()));
        match word {
            "hover" => {
                let region = self.region_named(rest)?;
                self.hover_region(region);
            }
            "select" => {
                let picked = self.picked_named(rest)?;
                let point = self.point_of(self.volume_of(&picked));
                self.start_edit(picked, point);
                if self.session.borrow().edit().is_none() {
                    return Err(format!("no edit started from '{rest}'"));
                }
                self.check_menu_now();
            }
            "click" => {
                // A click on a part's face: the part selected, its card
                // offering to add onto that face.
                let Picked::Face(face) = self.picked_named(rest)? else {
                    return Err("write it as click face <part>.<face>".to_owned());
                };
                self.click_face(face);
            }
            "choose" => {
                let index = self
                    .session
                    .borrow()
                    .edit()
                    .ok_or("nothing is selected to choose from")?
                    .menu()
                    .iter()
                    .position(|candidate| entry_matches(candidate, rest))
                    .ok_or_else(|| format!("the menu has no entry '{rest}'"))?;
                self.send_edit(EditCommand::Choose(index));
            }
            "bind" => {
                let picked = self.picked_named(rest)?;
                self.send_edit(EditCommand::Bind(picked));
            }
            "set" => {
                let (name, value) = rest
                    .split_once('=')
                    .ok_or("write it as set <name> = <value>")?;
                let name = parse_name(name.trim()).map_err(|error| error.0)?;
                let value = parse_value(value.trim()).map_err(|error| error.0)?;
                self.send_edit(EditCommand::Set { name, value });
            }
            "name" => {
                let name = parse_name(rest).map_err(|error| error.0)?;
                self.send_edit(EditCommand::Rename(name));
            }
            "fix" => {
                let numbers: Vec<usize> = rest
                    .split_whitespace()
                    .map(|number| number.parse::<usize>().map_err(|error| error.to_string()))
                    .collect::<Result<_, _>>()?;
                let [problem, fix] = numbers.as_slice() else {
                    return Err("write it as fix <problem> <fix>, both from 1".to_owned());
                };
                self.send_edit(EditCommand::ApplyFix {
                    problem: problem.checked_sub(1).ok_or("problems count from 1")?,
                    fix: fix.checked_sub(1).ok_or("fixes count from 1")?,
                });
            }
            "add" | "commit" => self.send_edit(EditCommand::Add),
            "cancel" => self.send_edit(EditCommand::Cancel),
            _ => return Err(format!("'{word}' isn't a step of an edit script")),
        }
        Ok(())
    }

    /// A region of the design shown, by its name.
    fn region_named(&self, name: &str) -> Result<RegionId, String> {
        let session = self.session.borrow();
        let (_, graph) = session.drawable().ok_or("nothing is drawn yet")?;
        graph
            .regions()
            .iter()
            .find(|region| region.name == name)
            .map(|region| region.id)
            .ok_or_else(|| format!("the design has no space called '{name}'"))
    }

    /// A part of the design shown, by its name.
    fn part_named(&self, name: &str) -> Result<PartId, String> {
        let session = self.session.borrow();
        let (design, _) = session.drawable().ok_or("nothing is drawn yet")?;
        design
            .parts
            .iter()
            .find(|part| part.name == name)
            .map(|part| part.id)
            .ok_or_else(|| format!("the design has no part called '{name}'"))
    }

    /// A selection written as `region <name>`, `face <part>.<face>` or
    /// `between <part> <part>`.
    fn picked_named(&self, text: &str) -> Result<Picked, String> {
        let words: Vec<&str> = text.split_whitespace().collect();
        match words.as_slice() {
            ["region", name] => Ok(Picked::Region(self.region_named(name)?)),
            ["face", face] => {
                let (part, face) = face
                    .rsplit_once('.')
                    .ok_or("write a face as <part>.<face>")?;
                let face = Face::from_word(face).ok_or_else(|| format!("'{face}' isn't a face"))?;
                Ok(Picked::Face(PartFace {
                    part: self.part_named(part)?,
                    face,
                }))
            }
            ["between", first, second] => {
                FacingPair::new(self.part_named(first)?, self.part_named(second)?)
                    .map(Picked::Between)
                    .map_err(|error| error.to_string())
            }
            _ => Err(format!(
                "'{text}' isn't a selection: write region <name>, face <part>.<face> or between <part> <part>"
            )),
        }
    }

    /// Where a selection is in the scene, to open its menu beside.
    fn volume_of(&self, picked: &Picked) -> Option<Aabb> {
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (_, graph) = session.drawable()?;
        let posed = state.posed.as_ref()?;
        let parts: Vec<PartId> = match picked {
            Picked::Region(region) => return graph.region_volume(posed, *region),
            Picked::Face(face) => vec![face.part],
            Picked::Between(pair) => vec![pair.first(), pair.second()],
            Picked::Group(parts) => parts.iter().copied().collect(),
        };
        let nodes: Vec<_> = graph
            .node_ids()
            .filter(|id| {
                graph.node(*id).is_some_and(
                    |node| matches!(node.item, Item::Part(part) if parts.contains(&part)),
                )
            })
            .collect();
        graph.bounds_of(posed, &nodes)
    }
}
