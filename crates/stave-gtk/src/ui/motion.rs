//! Things that move on the 3D view: the "Open it" slider, a posed
//! problem's swing, and Pull apart with its play button.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use stave_app::problems::MotionCard;
use stave_app::session::{Command, Explode};
use stave_app::{Contact, view};
use stave_model::solved::AssemblyId;

use super::widgets::narrow;
use super::{EXPLODE_PER_SECOND, Opening, ShownProblem, Window, view_tools};

impl Window {
    /// Offers the motion bar to open the selection's door or drawer.
    pub(super) fn update_opening(self: &Rc<Self>, assembly: Option<AssemblyId>) {
        let found = {
            let session = self.session.borrow();
            assembly.and_then(|id| {
                let (design, graph) = session.drawable()?;
                let range = graph.motion_range(id)?;
                let name = design.assemblies.get(id.index())?.name.clone();
                Some((id, range, name))
            })
        };
        let amount = {
            let mut state = self.state.borrow_mut();
            if state.focus.is_some() {
                return;
            }
            let keep = state
                .opening
                .filter(|o| Some(o.assembly) == found.as_ref().map(|(id, _, _)| *id));
            state.opening = keep.or_else(|| {
                found.as_ref().map(|(id, range, _)| Opening {
                    assembly: *id,
                    amount: 0.0,
                    turns: range.turns,
                })
            });
            state.updating_controls = true;
            state.opening.map_or(0.0, |o| o.amount)
        };
        match &found {
            Some((_, range, name)) => {
                let (full, value, end) = if range.turns {
                    let degrees = range.full.to_degrees();
                    (
                        degrees,
                        amount.to_degrees(),
                        format!("{}°", degrees.round()),
                    )
                } else {
                    (range.full, amount, format!("{} mm", range.full.round()))
                };
                self.widgets.motion.show(Some((
                    name,
                    "closed",
                    &end,
                    f64::from(full),
                    f64::from(value),
                )));
            }
            None => self.widgets.motion.show(None),
        }
        self.state.borrow_mut().updating_controls = false;
        self.repose();
    }

    /// The motion bar moved: the selection opening.
    pub(super) fn motion_moved(self: &Rc<Self>, value: f64) {
        {
            let mut state = self.state.borrow_mut();
            if state.updating_controls {
                return;
            }
            let value = narrow(value);
            if let Some(opening) = state.opening.as_mut() {
                opening.amount = if opening.turns {
                    value.to_radians()
                } else {
                    value
                };
            }
        }
        self.repose();
        self.request_render();
    }

    /// The problem card's slider moved: swings the problem's motion.
    pub(super) fn problem_moved(self: &Rc<Self>, value: f64) {
        let shown = {
            let mut state = self.state.borrow_mut();
            let Some(problem) = state.problem.as_mut() else {
                return;
            };
            let value = narrow(value);
            let amount = if problem.turns {
                value.to_radians()
            } else {
                value
            };
            problem.amount = Some(amount);
            problem.card.clone().map(|card| (card, amount))
        };
        if let Some((card, amount)) = &shown {
            self.widgets.problem_card.show_value(card, *amount);
            self.show_posed_status(card, *amount);
        }
        self.repose();
        self.request_render();
    }

    /// "Posed at contact · door at 0.6°" at the top of the view.
    pub(super) fn show_posed_status(&self, card: &MotionCard, amount: f32) {
        self.show_hint(&format!(
            "Posed at contact · {} at {}",
            card.name,
            card.amount_text(amount)
        ));
    }

    /// Shows the controls for a posed problem (its motion card and the
    /// shared-wood key in place of the view toolbar), or the usual ones.
    pub(super) fn show_problem_controls(&self, problem: Option<(&MotionCard, f32, bool)>) {
        let widgets = &self.widgets;
        match problem {
            Some((card, amount, _)) => widgets.problem_card.show(Some(card), amount),
            None => widgets.problem_card.show(None, 0.0),
        }
        let posed = problem.is_some();
        let shared = problem.is_some_and(|(_, _, shared)| shared);
        let focused = self.state.borrow().focus.is_some();
        widgets.contact_key.set_visible(shared && !focused);
        widgets.offer_view_tools(!posed && !focused);
        if !posed && !focused {
            self.show_hint("");
        }
    }

    /// Poses the design where a problem happens and frames its parts.
    pub(super) fn pose_problem(self: &Rc<Self>, contact: Contact, card: Option<MotionCard>) {
        let highlight = {
            let session = self.session.borrow();
            session
                .drawable()
                .map(|(_, graph)| view::highlight(graph, &contact))
        };
        let Some(highlight) = highlight else {
            return;
        };
        let shared = !highlight.volumes.is_empty();
        let turns = card.as_ref().is_none_or(|c| c.turns);
        {
            let mut state = self.state.borrow_mut();
            state.opening = None;
            state.problem = Some(ShownProblem {
                contact,
                highlight,
                card: card.clone(),
                amount: None,
                turns,
            });
        }
        self.widgets.motion.show(None);
        self.repose();
        {
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            let aspect = Self::aspect(&state);
            let framed = session.drawable().and_then(|(_, graph)| {
                let nodes = &state.problem.as_ref()?.highlight.nodes;
                let bounds = graph.bounds_of(state.posed.as_ref()?, nodes)?;
                Some(state.orbit?.framed(bounds, aspect))
            });
            if framed.is_some() {
                state.orbit = framed;
            }
        }
        match &card {
            Some(card) => {
                self.show_problem_controls(Some((card, card.shown, shared)));
                self.show_posed_status(card, card.shown);
            }
            None => {
                self.widgets.problem_card.show(None, 0.0);
                self.widgets.contact_key.set_visible(shared);
                self.widgets.offer_view_tools(false);
                if shared {
                    self.show_hint("Posed at contact");
                }
            }
        }
    }

    /// "Pull apart" switched on (all the way apart) or off (together).
    pub(super) fn pull_apart(self: &Rc<Self>, on: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.updating_controls || state.pull_apart == on {
                return;
            }
            state.pull_apart = on;
            state.exploding = false;
        }
        self.dispatch(Command::SetExplode(Explode::clamped(if on {
            1.0
        } else {
            0.0
        })));
        self.show_view_toolbar();
    }

    /// Shows the view toolbar as the session and the window have it.
    pub(super) fn show_view_toolbar(&self) {
        let explode = self.session.borrow().explode().get();
        let (on, playing) = {
            let mut state = self.state.borrow_mut();
            state.pull_apart |= explode > 0.0;
            state.updating_controls = true;
            (state.pull_apart, state.exploding)
        };
        self.widgets.view_toolbar.show(on, explode, playing);
        self.state.borrow_mut().updating_controls = false;
    }

    /// The pull apart play button: plays from together to apart (from the
    /// start when it is apart already), or pauses.
    pub(super) fn toggle_play_explode(self: &Rc<Self>) {
        let playing = {
            let mut state = self.state.borrow_mut();
            state.exploding = !state.exploding;
            state.exploding
        };
        if playing && self.session.borrow().explode().get() >= 1.0 {
            self.dispatch(Command::SetExplode(Explode::clamped(0.0)));
        }
        self.show_view_toolbar();
        if !playing {
            return;
        }
        let window = Rc::downgrade(self);
        let last: std::cell::Cell<Option<i64>> = std::cell::Cell::new(None);
        self.widgets.viewport.add_tick_callback(move |_, clock| {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !window.state.borrow().exploding {
                return glib::ControlFlow::Break;
            }
            let now = clock.frame_time();
            let seconds = last.get().map_or(0.0, |before| {
                f64::from(i32::try_from(now - before).unwrap_or(0)) / 1e6
            });
            last.set(Some(now));
            let explode = window.session.borrow().explode().get();
            let next = (explode + narrow(seconds * EXPLODE_PER_SECOND)).min(1.0);
            let done = next >= 1.0;
            if done {
                window.state.borrow_mut().exploding = false;
            }
            window.dispatch(Command::SetExplode(Explode::clamped(next)));
            if done {
                window.show_view_toolbar();
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    pub(super) fn explode_changed(self: &Rc<Self>) {
        self.show_view_toolbar();
        let explode = self.session.borrow().explode().get();
        let quiet = {
            let state = self.state.borrow();
            state.focus.is_none() && state.problem.is_none()
        };
        if quiet && self.session.borrow().selection().is_none() {
            self.show_hint(&if explode > 0.0 {
                format!("Pulled apart · {}%", view_tools::percent(explode))
            } else {
                String::new()
            });
        }
        self.repose();
        self.request_render();
    }
}
