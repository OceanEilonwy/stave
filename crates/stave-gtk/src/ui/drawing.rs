//! Drawing the 3D view: what each frame shows (the poses, the looks of
//! the selection and the hover), picking under the pointer, and
//! projecting points to the screen.

use stave_app::i18n::tr;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::session::{Command, Selection};
use stave_app::{focus, view};
use stave_model::solved::AssemblyId;
use stave_scene::{NodeId, Orbit};

use crate::render::{Backdrop, Frame, Highlighted};

use super::viewport::Shown;
use super::{EDGE, State, Window, edit_view};

impl Window {
    /// The 3D view's area behind its cards takes the stage colour, so a
    /// page laid out around the view (hardware focus) matches it.
    pub(super) fn show_backdrop(&self, dark: bool) {
        for page in [&self.widgets.stage, &self.widgets.empty.root] {
            if dark {
                page.add_css_class("dark");
            } else {
                page.remove_css_class("dark");
            }
        }
        // What only the dark style changes in the sheet (the code blocks)
        // hangs off the window: a CSS media query doesn't see a style
        // chosen in the app's menu.
        if dark {
            self.widgets.window.add_css_class("stave-dark");
        } else {
            self.widgets.window.remove_css_class("stave-dark");
        }
    }

    pub(super) fn scene_changed(self: &Rc<Self>) {
        let (titles, index) = {
            let session = self.session.borrow();
            let titles: Vec<String> = session
                .drawable()
                .map(|(_, g)| g.scenes.iter().map(|s| s.title.clone()).collect())
                .unwrap_or_default();
            (titles, session.scene_index())
        };
        self.widgets.header.show_scenes(&titles, index);
        self.state.borrow_mut().problem = None;
        self.show_problem_controls(None);
        self.repose();
        self.request_render();
    }

    /// Poses the design for what the window shows: focus mode, a problem
    /// at its contact, or the current scene pulled apart. In focus mode,
    /// returns its step and status for the panel.
    pub(super) fn repose(&self) -> Option<(Option<String>, String)> {
        let session = self.session.borrow();
        let (_, graph) = session.drawable()?;
        let entry = session.scene_entry()?;
        let explode = session.explode().get();
        let mut state = self.state.borrow_mut();
        let State {
            posed,
            problem,
            opening,
            focus,
            ..
        } = &mut *state;
        if let Some(focus) = focus.as_mut() {
            // From above, the section cuts the receiving part open: it is
            // drawn solid.
            let options = focus::Options {
                xray: focus.options.xray && !focus.plan,
                ..focus.options
            };
            let frame = focus.focus.frame(graph, entry, focus.progress, &options);
            *posed = Some(frame.posed);
            focus.looks = frame.looks;
            focus.overlay = frame.overlay;
            return Some((frame.step, frame.status));
        }
        let at_contact = problem
            .as_ref()
            .and_then(|problem| view::pose_at_contact(graph, &problem.contact, problem.amount));
        *posed = Some(at_contact.unwrap_or_else(|| {
            let overrides: Vec<(AssemblyId, f32)> =
                opening.iter().map(|o| (o.assembly, o.amount)).collect();
            graph.pose_with(entry, &overrides, explode)
        }));
        None
    }

    pub(super) fn pick(self: &Rc<Self>, x: f64, y: f64) {
        if self.edit_click(x, y) {
            return;
        }
        let hit = {
            let session = self.session.borrow();
            let state = self.state.borrow();
            let viewport = &self.widgets.viewport;
            session
                .drawable()
                .zip(state.orbit)
                .zip(state.posed.as_ref())
                .and_then(|(((_, graph), orbit), posed)| {
                    let ray = orbit.ray(
                        crate::convert::f32_of(x),
                        crate::convert::f32_of(y),
                        crate::convert::f32_of_int(viewport.width()),
                        crate::convert::f32_of_int(viewport.height()),
                    );
                    graph.pick(posed, ray)
                })
        };
        let selection = hit.map(|hit| Selection::of_item(hit.item));
        self.dispatch(Command::Select(selection));
    }

    /// The stage colour for the current style.
    pub(super) fn backdrop(&self) -> Backdrop {
        if adw::StyleManager::default().is_dark() {
            Backdrop::Dark
        } else {
            Backdrop::Light
        }
    }

    /// Draws on the next frame; several requests before then draw once.
    pub(super) fn request_render(self: &Rc<Self>) {
        // The Model hidden behind an open file's tab draws nothing, so a
        // screenshot asked for is taken without it.
        if self.widgets.window.is_mapped() && !self.widgets.viewport.is_mapped() {
            self.capture_if_asked();
        }
        {
            let mut state = self.state.borrow_mut();
            if state.render_queued {
                return;
            }
            state.render_queued = true;
        }
        let window = self.clone();
        self.widgets.viewport.add_tick_callback(move |_, _| {
            window.render_now();
            glib::ControlFlow::Break
        });
    }

    /// Draws a frame. How each node looks outside focus mode: choosing
    /// something only outlines it, so the rest stays solid and clicking
    /// around never switches the view between solid and see-through; with
    /// X-ray on everything but the selection is see-through; a posed
    /// problem fades what it doesn't involve (`frame_lists.rs`).
    pub(super) fn render_now(self: &Rc<Self>) {
        let backdrop = self.backdrop();
        let ghost = self.render_ghost(backdrop);
        let rendered = if let Some(rendered) = ghost {
            rendered
        } else {
            self.refresh_frame_lists();
            let mut lists = self.frame_lists.borrow_mut();
            let session = self.session.borrow();
            let mut state = self.state.borrow_mut();
            state.render_queued = false;
            let (width, height) = state.size;
            let State {
                posed,
                orbit,
                renderer,
                problem,
                focus,
                ..
            } = &mut *state;
            let (Some((_, graph)), Some(orbit), Some(posed), Ok(renderer)) =
                (session.drawable(), *orbit, posed.as_ref(), renderer)
            else {
                return;
            };
            let focus = focus.as_ref();
            let plan = focus
                .filter(|f| f.plan)
                .and_then(|f| f.focus.plan().copied());
            let orbit = match plan {
                Some(plan) => {
                    Orbit::plan(plan.bounds, crate::render::aspect(width, height), plan.yaw)
                }
                None => orbit,
            };
            // The shared wood is drawn only where it was measured; moved by
            // the slider, the parts stay tinted.
            let highlight = problem.as_ref().map(|problem| Highlighted {
                nodes: &problem.highlight.nodes,
                volumes: match problem.amount {
                    None => &problem.highlight.volumes,
                    Some(_) => &[],
                },
            });
            // The space the pointer or an edit is on, lit, and what covers
            // it see-through.
            let marks = if focus.is_none() {
                self.edit_marks(&session, graph, posed, &orbit)
            } else {
                edit_view::EditMarks::default()
            };
            let count = graph.node_ids().count();
            // Outside focus mode, a selected joint member shows through
            // the parts it sits in.
            let (selection, outside, looks) = lists.for_frame(&marks.covering, count);
            renderer.submit(&Frame {
                scene: graph,
                posed,
                orbit,
                selection: if focus.is_some() { &[] } else { selection },
                highlight,
                section: plan
                    .map(|p| p.section)
                    .or_else(|| session.scene_entry().and_then(|entry| entry.section)),
                looks: focus.map(|f| f.looks.as_slice()).or(outside).or(looks),
                overlay: focus.map(|f| &f.overlay),
                tint_cut_faces: focus.is_none_or(|f| f.options.tint_cut_faces),
                lit: Some(&marks.lit),
                tints: None,
                backdrop,
                width,
                height,
            })
        };
        let ticket = match rendered {
            Ok(ticket) => ticket,
            Err(error) => {
                let message = format!("{error:#}");
                self.show_status(&tr("Can't draw in 3D"), &message);
                return;
            }
        };
        // The frame is shown once the GPU has it ready, on a later tick:
        // the main loop never waits for the GPU. A newer frame ready first
        // is shown instead, and this ticket is done with.
        let window = self.clone();
        self.widgets.viewport.add_tick_callback(move |_, _| {
            let (ready, settled) = {
                let mut state = window.state.borrow_mut();
                let Ok(renderer) = &mut state.renderer else {
                    return glib::ControlFlow::Break;
                };
                (renderer.take_ready(), renderer.settled(ticket))
            };
            match ready {
                Ok(Some(frame)) => {
                    let shown = crate::render::profile::time("gtk texture", || {
                        window.widgets.viewport.show_live(frame)
                    });
                    match shown {
                        Shown::Yes => crate::render::profile::frame_shown(),
                        Shown::No => {}
                        Shown::Refused => window.request_render(),
                    }
                    window.show_pins();
                    window.place_ghost_verdict();
                    window.place_grips();
                    // Save the window once a frame of the view's real size
                    // is up.
                    let settled = {
                        let state = window.state.borrow();
                        state.size.0 > 1 && state.size == window.widgets.viewport.device_size()
                    };
                    if settled {
                        // An edit script runs first; the frame it asks for
                        // is the one saved.
                        if window.run_edit_script() {
                            window.request_render();
                        } else {
                            window.capture_if_asked();
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    let message = format!("{error:#}");
                    window.show_status(&tr("Can't draw in 3D"), &message);
                }
            }
            if settled {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Where a set of nodes shows in the 3D view, in logical pixels from
    /// its top left (the middle of their bounds as posed); `None` when it
    /// can't be worked out (nothing drawn yet, behind the camera).
    pub(super) fn screen_point(&self, nodes: &[NodeId]) -> Option<(f32, f32)> {
        let session = self.session.borrow();
        let state = self.state.borrow();
        let (_, graph) = session.drawable()?;
        let bounds = graph.bounds_of(state.posed.as_ref()?, nodes)?;
        self.project(&state, bounds.centre())
    }

    /// How much of the 3D view's right side the camera column takes (with
    /// the space beside it), in logical pixels: what the editing's cards
    /// and labels keep clear of at the right.
    pub(super) fn right_column(&self) -> i32 {
        let camera = &self.widgets.camera.root;
        if camera.is_visible() {
            camera.width().saturating_add(EDGE)
        } else {
            0
        }
    }

    /// Where a point of the scene shows in the 3D view, in logical pixels.
    pub(super) fn project(&self, state: &State, point: glam::Vec3) -> Option<(f32, f32)> {
        let viewport = &self.widgets.viewport;
        state.orbit?.project(
            point,
            crate::convert::f32_of_int(viewport.width()),
            crate::convert::f32_of_int(viewport.height()),
        )
    }
}

/// A rendered image as a texture for GTK.
pub(super) fn texture(image: crate::render::Image) -> Option<gdk::Texture> {
    let stride = usize::try_from(image.width).ok()?.checked_mul(4)?;
    let width = i32::try_from(image.width).ok()?;
    let height = i32::try_from(image.height).ok()?;
    let bytes = glib::Bytes::from_owned(image.rgba);
    // Alpha is 1 everywhere, so the bytes are premultiplied already: the
    // format GTK draws without converting.
    let texture = gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::R8g8b8a8Premultiplied,
        &bytes,
        stride,
    );
    Some(texture.upcast())
}
