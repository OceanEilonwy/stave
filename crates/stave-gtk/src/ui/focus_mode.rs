//! Focus mode: opening it on a joint or a piece of hardware, framing
//! and laying out its view, its step player and the pictures beside it.

use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use stave_app::focus;
use stave_app::focus::Target;
use stave_scene::{Orbit, Overlay};

use crate::render::Frame;

use super::drawing::texture;
use super::widgets::narrow;
use super::{FocusState, PICTURES_DELAY, PLAY_PER_SECOND, State, Window, focus_stage};

impl Window {
    /// Opens or leaves focus mode to match the session.
    pub(super) fn focus_changed(self: &Rc<Self>) {
        let wanted = self.session.borrow().focus();
        let current = self.state.borrow().focus.as_ref().map(|f| f.focus.target);
        if wanted == current {
            return;
        }
        match wanted {
            Some(target) => self.open_focus(target),
            None => self.leave_focus(),
        }
    }

    pub(super) fn open_focus(self: &Rc<Self>, target: Target) {
        let found = {
            let session = self.session.borrow();
            session.shown().and_then(|shown| {
                let (design, graph) = shown.drawable()?;
                let focus = focus::open(&shown.opened, design, graph, target)?;
                let name = focus
                    .motion()
                    .and_then(|(assembly, _)| design.assemblies.get(assembly.index()))
                    .map(|assembly| assembly.name.clone());
                Some((focus, name))
            })
        };
        let Some((found, moving_name)) = found else {
            return;
        };
        let start = self.state.borrow_mut().focus_start.take();
        let (start_plan, start_step) = start.unwrap_or((None, None));
        let progress = start_step.unwrap_or(if found.assembles() { 0.0 } else { 0.4 });
        let plan = found.plan().is_some() && start_plan.unwrap_or(found.prefers_plan());
        {
            let mut state = self.state.borrow_mut();
            state.problem = None;
            state.opening = None;
            state.exploding = false;
            state.focus = Some(FocusState {
                focus: found,
                progress,
                options: focus::Options::default(),
                plan,
                playing: false,
                looks: Vec::new(),
                overlay: Overlay::default(),
            });
        }
        self.widgets.motion.show(None);
        self.show_problem_controls(None);
        self.layout_for_focus(moving_name.as_deref());
        self.hide_pins();
        let texts = self.repose();
        self.frame_focus();
        {
            let state = self.state.borrow();
            self.widgets
                .inspector
                .focus
                .show(state.focus.as_ref().map(|f| &f.focus));
        }
        // Setting the check boxes calls their handlers, which look at the
        // state: nothing may be borrowed here.
        self.state.borrow_mut().updating_controls = true;
        let panel = &self.widgets.inspector.focus;
        panel.xray.set_active(true);
        panel.tint.set_active(true);
        panel.fade.set_active(true);
        self.widgets.plan_switch.show(plan);
        self.state.borrow_mut().updating_controls = false;
        self.widgets.inspector.show_focus();
        let assembles = self
            .state
            .borrow()
            .focus
            .as_ref()
            .is_some_and(|f| f.focus.assembles());
        self.show_hint(if assembles {
            "Focused on one joint · Esc to go back"
        } else {
            "Focused on one piece of hardware · Esc to go back"
        });
        self.show_focus_texts(texts);
        self.show_progress();
        self.render_pictures();
        self.request_render();
    }

    /// Frames the joint or the hardware (from the direction focus asks
    /// for).
    pub(super) fn frame_focus(&self) {
        let mut state = self.state.borrow_mut();
        let aspect = Self::aspect(&state);
        let framed = state
            .focus
            .as_ref()
            .and_then(|f| f.focus.zone())
            .zip(state.orbit);
        let view = state.focus.as_ref().and_then(|f| f.focus.view());
        if let Some((zone, orbit)) = framed {
            let orbit = match view {
                Some((yaw, pitch)) => Orbit {
                    yaw,
                    pitch,
                    ..orbit
                },
                None => orbit,
            };
            state.orbit = Some(orbit.framed(zone, aspect));
        }
    }

    /// Lays the 3D view's area out for focus mode (`Some` focus) or for
    /// the Parts view: a joint gets its step player over the view; hardware
    /// gets a page with a line over the view, three small views and the
    /// swing card.
    pub(super) fn layout_for_focus(&self, moving_name: Option<&str>) {
        let state = self.state.borrow();
        let focus = state.focus.as_ref();
        let widgets = &self.widgets;
        let joint = focus.is_some_and(|f| f.focus.assembles());
        let hardware = focus.filter(|f| !f.focus.assembles());
        let page = hardware.is_some();
        if page {
            widgets.stage.add_css_class("page");
        } else {
            widgets.stage.remove_css_class("page");
        }
        widgets.step_card.root.set_visible(joint);
        let swing = hardware.and_then(|f| f.focus.motion());
        widgets.swing_card.root.set_visible(swing.is_some());
        if let Some((_, range)) = swing {
            widgets
                .swing_card
                .show_name(moving_name.unwrap_or("it"), range.turns);
        }
        widgets.caption.root.set_visible(page);
        widgets.pictures.root.set_visible(false);
        widgets
            .plan_switch
            .root
            .set_visible(focus.is_some_and(|f| f.focus.plan().is_some()));
        widgets.camera.root.set_visible(focus.is_none());
        widgets.offer_view_tools(focus.is_none());
        drop(state);
        self.show_caption();
    }

    /// The line over hardware focus's view: from above or in 3D.
    pub(super) fn show_caption(&self) {
        let state = self.state.borrow();
        let Some(focus) = state.focus.as_ref() else {
            return;
        };
        let (text, hint) = match focus.focus.plan_caption().filter(|_| focus.plan) {
            Some(caption) => (caption.to_owned(), "back ↑ · front ↓"),
            None => (
                format!("In 3D around {}", focus.focus.title),
                "drag to turn · scroll to zoom",
            ),
        };
        self.widgets.caption.show(&text, hint);
    }

    pub(super) fn leave_focus(self: &Rc<Self>) {
        self.state.borrow_mut().focus = None;
        self.widgets.inspector.focus.show(None);
        self.layout_for_focus(None);
        self.show_hint("");
        self.repose();
        self.selection_changed();
    }

    pub(super) fn show_focus_texts(&self, texts: Option<(Option<String>, String)>) {
        if let Some((step, status)) = texts {
            self.widgets.step_card.show_frame(step.as_deref(), &status);
        }
    }

    /// Moves the step player's and the swing card's sliders to the
    /// progress, with the swing's reading and the small view nearest it
    /// ringed.
    pub(super) fn show_progress(&self) {
        let state = self.state.borrow();
        let Some(focus) = state.focus.as_ref() else {
            return;
        };
        let progress = f64::from(focus.progress);
        let widgets = &self.widgets;
        widgets.step_card.scrub.set_value(progress);
        widgets.swing_card.scrub.set_value(progress);
        focus_stage::show_playing(
            &widgets.step_card.play,
            &widgets.step_card.play_icon,
            focus.playing,
            "Play the assembly",
        );
        focus_stage::show_playing(
            &widgets.swing_card.play,
            &widgets.swing_card.play_icon,
            focus.playing,
            "Play the swing",
        );
        if let Some((_, range)) = focus.focus.motion() {
            widgets
                .swing_card
                .show_value(&stave_scene::focus::amount_text(
                    range,
                    focus.progress * range.full,
                ));
        }
        let nearest = focus
            .focus
            .key_poses
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.progress - focus.progress)
                    .abs()
                    .total_cmp(&(b.progress - focus.progress).abs())
            })
            .map(|(index, _)| index);
        widgets.pictures.show_current(nearest);
    }

    /// The scrub moved, or a toggle changed: poses again.
    pub(super) fn update_focus(self: &Rc<Self>, change: impl FnOnce(&mut FocusState)) {
        {
            // The window sets the controls with nothing borrowed and with
            // `updating_controls` up; a borrow held here would be a bug.
            let mut state = self.state.borrow_mut();
            if state.updating_controls {
                return;
            }
            let Some(focus) = state.focus.as_mut() else {
                return;
            };
            change(focus);
        }
        let texts = self.repose();
        self.show_focus_texts(texts);
        self.show_progress();
        self.show_caption();
        self.request_render();
    }

    /// The play button: plays from where it is (from the start when it is
    /// at the end), or pauses.
    pub(super) fn toggle_play_focus(self: &Rc<Self>) {
        let playing = {
            let mut state = self.state.borrow_mut();
            let Some(focus) = state.focus.as_mut() else {
                return;
            };
            focus.playing = !focus.playing;
            if focus.playing && focus.progress >= 1.0 {
                focus.progress = 0.0;
            }
            focus.playing
        };
        self.show_progress();
        if !playing {
            return;
        }
        let window = Rc::downgrade(self);
        let last: std::cell::Cell<Option<i64>> = std::cell::Cell::new(None);
        self.widgets.viewport.add_tick_callback(move |_, clock| {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = clock.frame_time();
            let seconds = last.get().map_or(0.0, |before| {
                f64::from(i32::try_from(now - before).unwrap_or(0)) / 1e6
            });
            last.set(Some(now));
            let done = {
                let mut state = window.state.borrow_mut();
                let Some(focus) = state.focus.as_mut() else {
                    return glib::ControlFlow::Break;
                };
                if !focus.playing {
                    return glib::ControlFlow::Break;
                }
                focus.progress = (focus.progress + narrow(seconds * PLAY_PER_SECOND)).min(1.0);
                let done = focus.progress >= 1.0;
                if done {
                    focus.playing = false;
                }
                done
            };
            let texts = window.repose();
            window.show_focus_texts(texts);
            window.show_progress();
            window.request_render();
            if done {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Renders hardware focus's small views again shortly, once their
    /// size has stopped changing.
    pub(super) fn schedule_pictures(self: &Rc<Self>) {
        let window = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(PICTURES_DELAY, move || {
            if let Some(window) = window.upgrade() {
                window.state.borrow_mut().pictures_timer = None;
                window.render_pictures();
            }
        });
        if let Some(old) = self.state.borrow_mut().pictures_timer.replace(timer) {
            old.remove();
        }
    }

    /// Renders hardware focus's small views offscreen, at fixed angles
    /// (closed, part way, fully open), in the view from above when there
    /// is one, each at its slot's size in device pixels. Before the slots
    /// have room they are only laid out: their first size renders them.
    pub(super) fn render_pictures(self: &Rc<Self>) {
        let captions: Vec<String> = {
            let state = self.state.borrow();
            match state.focus.as_ref() {
                Some(focus) if !focus.focus.assembles() => focus
                    .focus
                    .key_poses
                    .iter()
                    .map(|pose| pose.caption.clone())
                    .collect(),
                Some(_) | None => Vec::new(),
            }
        };
        let window = Rc::downgrade(self);
        self.widgets.pictures.root.set_visible(!captions.is_empty());
        self.widgets.pictures.show(&captions, move |index| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let progress = window
                .state
                .borrow()
                .focus
                .as_ref()
                .and_then(|f| f.focus.key_poses.get(index).map(|pose| pose.progress));
            if let Some(progress) = progress {
                window.update_focus(|focus| {
                    focus.progress = progress;
                    focus.playing = false;
                });
            }
        });
        let Some((width, height)) = self.widgets.pictures.device_size() else {
            return;
        };
        let backdrop = self.backdrop();
        let textures = {
            let session = self.session.borrow();
            let (Some((_, graph)), Some(entry)) = (session.drawable(), session.scene_entry())
            else {
                return;
            };
            let mut state = self.state.borrow_mut();
            let State {
                focus,
                renderer,
                orbit,
                ..
            } = &mut *state;
            let (Some(focus), Ok(renderer), Some(orbit)) = (focus.as_ref(), renderer, *orbit)
            else {
                return;
            };
            if focus.focus.assembles() {
                return;
            }
            let mut textures = Vec::new();
            for pose in &focus.focus.key_poses {
                let frame = focus
                    .focus
                    .frame(graph, entry, pose.progress, &focus.options);
                let aspect = crate::render::aspect(width, height);
                let (orbit, section) = match focus.focus.plan() {
                    Some(plan) => (
                        Orbit::plan(plan.bounds, aspect, plan.yaw),
                        Some(plan.section),
                    ),
                    None => (
                        focus
                            .focus
                            .zone()
                            .map_or(orbit, |zone| orbit.framed(zone, aspect)),
                        entry.section,
                    ),
                };
                let image = renderer.render(&Frame {
                    scene: graph,
                    posed: &frame.posed,
                    orbit,
                    selection: &[],
                    highlight: None,
                    section,
                    looks: Some(&frame.looks),
                    overlay: None,
                    lit: None,
                    tints: None,
                    tint_cut_faces: focus.options.tint_cut_faces,
                    backdrop,
                    width,
                    height,
                });
                let Ok(image) = image else { continue };
                if let Some(texture) = texture(image) {
                    textures.push(texture);
                }
            }
            textures
        };
        self.widgets.pictures.show_textures(textures);
        self.show_progress();
    }
}
