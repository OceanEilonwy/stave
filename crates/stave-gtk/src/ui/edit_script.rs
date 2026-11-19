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
//! point face <part>.<face>             the next click, made on the model at that face's
//!                                      middle and picked as the pointer's click is
//! set <name> = <value>                 a setting typed on the card
//! options <name>                       the setting's list of options opened, each checked
//! put back                             the button that puts back a setting a refusal blames
//! name <name>                          the name retyped on the card
//! fix <problem> <fix>                  a fix's button, both counted from 1
//! add | cancel                         the card's buttons
//! save                                 Ctrl+S: every open file with unsaved changes saved
//! turn <degrees> [<pitch degrees>]     the view orbited: turned about the upright, and
//!                                      tipped to that height above the horizontal
//! ```
//!
//! `#` starts a comment. A step that can't be done is said on standard
//! error with its line, and the steps after it are skipped.

use gtk::prelude::ButtonExt;

use stave_app::edit::{FacingPair, PartFace, Picked};
use stave_app::editing::{EditCommand, entry_matches, parse_name, parse_value};
use stave_lang::placement::Face;
use stave_model::solved::{PartId, RegionId};
use stave_scene::{Aabb, Item};

use super::Window;

impl Window {
    /// Runs the edit script the command line gave, once. `true` when it
    /// ran (the window draws again before any screenshot is taken).
    pub(super) fn run_edit_script(&mut self) -> bool {
        let Some(path) = self.state.edit_script.take() else {
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
    fn edit_step(&mut self, step: &str) -> Result<(), String> {
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
                if self.session.edit().is_none() {
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
            "point" => {
                // A click on the model where the face shows, picked as
                // the pointer's own click is while the card asks for a
                // place (through a space to the wall behind it).
                let Picked::Face(face) = self.picked_named(rest)? else {
                    return Err("write it as point face <part>.<face>".to_owned());
                };
                let wanted = self.wanted();
                if !wanted.any() {
                    return Err("nothing on the card waits to be pointed at".to_owned());
                }
                let (x, y) = self
                    .face_point(face)
                    .ok_or_else(|| format!("'{rest}' isn't in view"))?;
                let picked = self
                    .binding_picked(x, y, wanted)
                    .ok_or_else(|| format!("a click on '{rest}' picks nothing"))?;
                if picked != Picked::Face(face) {
                    return Err(format!(
                        "a click on '{rest}' picks something else: {picked:?}"
                    ));
                }
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
            "options" => {
                self.open_setting_options(rest, "")?;
            }
            "choices" => {
                self.open_setting_choices(rest)?;
            }
            "search" => {
                let (setting, query) = rest
                    .split_once(char::is_whitespace)
                    .map_or((rest, ""), |(setting, query)| (setting, query.trim()));
                self.open_setting_options(setting, query.trim_matches('"'))?;
            }
            "default" => {
                let name = parse_name(rest).map_err(|error| error.0)?;
                self.send_edit(EditCommand::UseDefault(name));
            }
            "put" if rest == "back" => {
                let button = self
                    .editing
                    .card
                    .put_back_button()
                    .ok_or("the card offers nothing to put back")?;
                button.emit_clicked();
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
            // An Add with the file open in the editor leaves the change
            // unsaved in its buffer, as typing would; Ctrl+S writes it.
            "save" => {
                if !self.widgets.files.save_all() {
                    return Err("the open files couldn't be saved".to_owned());
                }
            }
            "turn" => {
                // The orbit drag between two clicks, so a face out of
                // sight can be brought round and pointed at.
                let degrees: Vec<f32> = rest
                    .split_whitespace()
                    .map(|word| word.parse::<f32>().map_err(|error| error.to_string()))
                    .collect::<Result<_, _>>()?;
                let (turn, pitch) = match degrees.as_slice() {
                    [turn] => (*turn, None),
                    [turn, pitch] if (-89.0..=89.0).contains(pitch) => (*turn, Some(*pitch)),
                    _ => {
                        return Err(
                            "write it as turn <degrees> [<pitch degrees, -89 to 89>]".to_owned()
                        );
                    }
                };
                {
                    let state = &mut self.state;
                    let orbit = state.orbit.as_mut().ok_or("nothing is drawn yet")?;
                    orbit.yaw = (orbit.yaw + turn.to_radians()).rem_euclid(std::f32::consts::TAU);
                    if let Some(pitch) = pitch {
                        orbit.pitch = pitch.to_radians();
                    }
                }
                self.request_render();
            }
            // The grips and the changes of the design share two words:
            // `pick` is a grip's while a follows grip's list waits, else a
            // click on an item; `drag count|boundary` is a repeat's
            // handle, any other `drag` a grip's.
            "pick" if !self.grip_choice_waits() => return self.change_step(word, rest),
            "drag" if matches!(rest.split_whitespace().next(), Some("count" | "boundary")) => {
                return self.change_step(word, rest);
            }
            _ => match self.grip_step(word, rest) {
                Some(result) => result?,
                None => return self.change_step(word, rest),
            },
        }
        Ok(())
    }

    /// A region of the design shown, by its name.
    fn region_named(&self, name: &str) -> Result<RegionId, String> {
        let session = &self.session;
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
        let session = &self.session;
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

    /// Where the middle of a part's face shows in the 3D view, a hair
    /// inside the part so a click there lands on that face.
    fn face_point(&self, face: PartFace) -> Option<(f64, f64)> {
        let session = &self.session;
        let state = &self.state;
        let (_, graph) = session.drawable()?;
        let nodes = graph.nodes_of(Item::Part(face.part));
        let bounds = graph.bounds_of(state.posed.as_ref()?, &nodes)?;
        let centre = bounds.centre();
        let half = (bounds.max - bounds.min) * 0.5;
        let outward = match face.face {
            Face::Top => glam::Vec3::Y,
            Face::Bottom => glam::Vec3::NEG_Y,
            Face::Left => glam::Vec3::NEG_X,
            Face::Right => glam::Vec3::X,
            Face::Front => glam::Vec3::Z,
            Face::Back => glam::Vec3::NEG_Z,
        };
        let point = centre + outward * (half.dot(outward.abs()) - 0.5);
        let (x, y) = self.project(state, point)?;
        Some((f64::from(x), f64::from(y)))
    }

    /// Where a selection is in the scene, to open its menu beside.
    fn volume_of(&self, picked: &Picked) -> Option<Aabb> {
        let session = &self.session;
        let state = &self.state;
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
