//! The problem pins on the 3D view: made once per check and selection,
//! placed on their parts each frame.

use std::rc::Rc;

use adw::prelude::*;
use stave_app::check::Checked;
use stave_app::problems::Mark;
use stave_app::session::{Command, ProblemIndex, Selection, View};
use stave_app::{problems, view};
use stave_scene::NodeId;

use super::{Window, edit_view};

impl Window {
    /// Takes every pin off the model.
    pub(super) fn hide_pins(&self) {
        let mut pins = self.pins.borrow_mut();
        for pin in pins.pins.drain(..) {
            self.widgets.design.remove_overlay(&pin.widget);
        }
        pins.made_for = None;
        pins.edge = None;
        self.widgets.edge_pin.set_visible(false);
    }

    /// Pins each problem where it happens (its shared wood, or the parts
    /// it is about), the first few; a problem whose place is off screen is
    /// named on the chip at the view's edge instead. A posed problem shows
    /// only its own pin; focus mode and an older design show none.
    ///
    /// Runs on every frame: the pins are made only when the result, the
    /// shown design or the posed problem changes; otherwise they are only
    /// moved to where their places now show.
    pub(super) fn show_pins(self: &Rc<Self>) {
        let key = {
            let session = self.session.borrow();
            let state = self.state.borrow();
            // An edit's menu, card and ghost take the view.
            let editing = session.edit().is_some() || session.change().is_some();
            let hidden = state.focus.is_some()
                || editing
                || session.showing_older()
                || session.view() != View::Design;
            match session.shown() {
                Some(shown) if !hidden => Some(PinsKey {
                    latest: session.latest().cloned(),
                    shown: Rc::clone(shown),
                    posed_problem: match session.selection() {
                        Some(Selection::Problem(index)) => Some(*index),
                        _ => None,
                    },
                }),
                _ => None,
            }
        };
        let Some(key) = key else {
            self.hide_pins();
            return;
        };
        let current = self
            .pins
            .borrow()
            .made_for
            .as_ref()
            .is_some_and(|made| made.same(&key));
        if !current {
            self.hide_pins();
            self.make_pins(key);
        }
        self.place_pins();
    }

    /// Makes the pins for the current problems (each one's highlight
    /// worked out once) and adds them to the view, not yet placed.
    pub(super) fn make_pins(self: &Rc<Self>, key: PinsKey) {
        let found: Vec<(ProblemIndex, Mark, String, PinAnchor)> = {
            let session = self.session.borrow();
            let Some((_, graph)) = session.drawable() else {
                return;
            };
            let problems = session.problems();
            problems::pins(problems, key.posed_problem, problems::PIN_LIMIT)
                .into_iter()
                .filter_map(|pinned| {
                    let problem = problems.get(pinned.index.0)?;
                    let highlight = view::highlight(graph, problem.contact.as_ref()?);
                    let anchor = match highlight.volumes.first() {
                        Some(volume) => PinAnchor::Volume(volume.centre()),
                        None => PinAnchor::Nodes(highlight.nodes),
                    };
                    Some((pinned.index, pinned.mark, problem.message.clone(), anchor))
                })
                .collect()
        };
        let pins: Vec<Pin> = found
            .into_iter()
            .map(|(index, mark, message, anchor)| {
                let widget = self.pin(index, mark, &message);
                widget.set_visible(false);
                self.widgets.design.add_overlay(&widget);
                Pin {
                    widget,
                    index,
                    mark,
                    message,
                    anchor,
                }
            })
            .collect();
        let mut state = self.pins.borrow_mut();
        state.pins = pins;
        state.made_for = Some(key);
    }

    /// Moves each pin to where its place shows now, hiding those whose
    /// place is off screen (or under a card) and naming the first of them
    /// on the chip at the view's edge.
    pub(super) fn place_pins(&self) {
        let (width, height) = (
            crate::convert::f32_of_int(self.widgets.viewport.width()),
            crate::convert::f32_of_int(self.widgets.viewport.height()),
        );
        let session = self.session.borrow();
        let state = self.state.borrow();
        let graph = session.drawable().map(|(_, graph)| graph);
        let mut pins = self.pins.borrow_mut();
        let mut first_off: Option<usize> = None;
        let mut off_count: usize = 0;
        for (place, pin) in pins.pins.iter().enumerate() {
            let point = match &pin.anchor {
                PinAnchor::Volume(centre) => self.project(&state, *centre),
                PinAnchor::Nodes(nodes) => graph
                    .zip(state.posed.as_ref())
                    .and_then(|(graph, posed)| graph.bounds_of(posed, nodes))
                    .and_then(|bounds| self.project(&state, bounds.centre())),
            };
            let spot = point
                .filter(|(x, y)| (0.0..width).contains(x) && (0.0..height).contains(y))
                .and_then(|(x, y)| {
                    // A hidden widget measures nothing.
                    pin.widget.set_visible(true);
                    // Clear of the cards and the parts list (edit_view).
                    let wanted = ((x - 12.0).max(0.0), (y - 24.0).max(0.0));
                    let size = edit_view::natural_size(pin.widget.upcast_ref());
                    let (dx, dy) = self.clear_spot(wanted, size)?;
                    Some((wanted.0 + dx, wanted.1 + dy))
                });
            match spot {
                Some((x, y)) => {
                    pin.widget
                        .set_margin_start(crate::convert::int_of_f32(x.round()).max(0));
                    pin.widget
                        .set_margin_top(crate::convert::int_of_f32(y.round()).max(0));
                    pin.widget.set_visible(true);
                }
                None => {
                    pin.widget.set_visible(false);
                    first_off.get_or_insert(place);
                    off_count = off_count.saturating_add(1);
                }
            }
        }
        let edge = first_off
            .and_then(|place| pins.pins.get(place))
            .map(|pin| (pin.index, off_count.saturating_sub(1)));
        if edge == pins.edge {
            return;
        }
        pins.edge = edge;
        let Some(pin) = first_off.and_then(|place| pins.pins.get(place)) else {
            self.widgets.edge_pin.set_visible(false);
            return;
        };
        let text = problems::off_screen_text(pin.mark, &pin.message, off_count.saturating_sub(1));
        self.widgets.edge_label.set_label(&text);
        self.widgets.edge_pin.set_visible(true);
        self.widgets
            .edge_pin
            .set_action_target_value(Some(&u64::try_from(pin.index.0).unwrap_or(0).to_variant()));
    }

    /// One pin: its mark's "!" and the problem's words, choosing it
    /// selects the problem (posed where it happens).
    pub(super) fn pin(self: &Rc<Self>, index: ProblemIndex, mark: Mark, message: &str) -> gtk::Box {
        let warning = mark == Mark::Warning;
        let button = gtk::Button::builder()
            .label("!")
            .css_classes(if warning {
                vec!["stave-pin", "warning"]
            } else {
                vec!["stave-pin"]
            })
            .tooltip_text(message)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&format!(
            "Show the problem: {message}"
        ))]);
        let label = gtk::Label::builder()
            .label(message)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(28)
            .css_classes(if warning {
                vec!["stave-pin-label", "warning"]
            } else {
                vec!["stave-pin-label"]
            })
            .build();
        let window = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Select(Some(Selection::Problem(index))));
            }
        });
        let pin = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .build();
        pin.append(&button);
        pin.append(&label);
        pin
    }
}

/// A problem pinned on the model: its widget, made once per result and
/// selection, and the place it is anchored to, projected again on every
/// frame.
pub(super) struct Pin {
    widget: gtk::Box,
    index: ProblemIndex,
    mark: Mark,
    message: String,
    anchor: PinAnchor,
}

/// Where a pin is anchored in the scene.
pub(super) enum PinAnchor {
    /// The middle of the problem's first shared volume (already posed).
    Volume(glam::Vec3),
    /// The middle of the bounds of the parts it is about, as posed now.
    Nodes(Vec<NodeId>),
}

/// The pins on the model and what they were made for: they are made again
/// only when the result, the shown design or the posed problem changes.
#[derive(Default)]
pub(super) struct Pins {
    made_for: Option<PinsKey>,
    pins: Vec<Pin>,
    /// The off-screen chip as last shown (first problem, how many more), so
    /// its words are written only when they change.
    edge: Option<(ProblemIndex, usize)>,
}

/// What a set of pins was made for.
pub(super) struct PinsKey {
    latest: Option<Rc<Checked>>,
    shown: Rc<Checked>,
    posed_problem: Option<ProblemIndex>,
}

impl PinsKey {
    fn same(&self, other: &Self) -> bool {
        let latest = match (&self.latest, &other.latest) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        };
        latest && Rc::ptr_eq(&self.shown, &other.shown) && self.posed_problem == other.posed_problem
    }
}
