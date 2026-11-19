//! The 3D view: the camera, drawing, the problems pinned on the model,
//! the view and scene chosen, pulling apart, X-ray, the style's backdrop
//! and exporting.

use super::add_action;
use crate::render::Ticket;
use crate::ui::view_tools::{CameraOutput, ViewToolbarOutput};
// The window's own items and imports, as the handlers had them in mod.rs.
use crate::ui::*;

/// What the rendering group is told.
#[derive(Debug)]
pub(in crate::ui) enum RenderingInput {
    /// Frames the selection, or everything.
    Frame,
    /// The Export View action: ask where to save.
    ExportView,
    /// Saves the view into this file.
    ExportTo(PathBuf),
    /// Saving the view finished.
    Exported {
        /// Where it was saved.
        path: PathBuf,
        /// Whether it was.
        saved: Result<(), stave_app::save_failure::SaveFailure>,
    },
    /// Shows a view (the switcher, the view actions).
    ShowView(View),
    /// The scene at this place in the scene menu was chosen.
    SceneChosen(usize),
    /// X-ray switched on or off.
    Xray(bool),
    /// Pull apart switched on or off.
    PullApart(bool),
    /// Pull apart's play button.
    PlayExplode,
    /// A frame while pulling apart plays, at this frame time.
    ExplodeTick(i64),
    /// Pull apart's slider moved.
    ExplodeSlid(f64),
    /// The front view button.
    LookFront,
    /// The turn around button.
    TurnAround,
    /// The style changed.
    StyleChanged {
        /// It is dark now.
        dark: bool,
    },
    /// The view has a new size, in device pixels.
    Resized {
        /// Its width.
        width: u32,
        /// Its height.
        height: u32,
    },
    /// A drag on the view began; a left drag also gives it the keyboard.
    DragBegun {
        /// The view takes the keyboard.
        grab_focus: bool,
    },
    /// A left drag moved this far from where it began: the camera turns.
    Orbited {
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// A right or middle drag moved this far: the view moves sideways.
    Panned {
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// A click on the view, in its logical pixels.
    Picked {
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// The scroll wheel turned this far.
    Zoomed(f64),
    /// Draws the view now.
    RenderNow,
    /// Shows the frame the GPU has ready.
    PollFrame(Ticket),
}

impl From<RenderingInput> for WindowInput {
    fn from(message: RenderingInput) -> WindowInput {
        WindowInput::Rendering(message)
    }
}

/// Wires the view's actions, the header's view and scene, the view's
/// toolbars, its gestures and keys, and the style to [`RenderingInput`]s.
pub(in crate::ui) fn wire(window: &mut Window) {
    let input = window.input();
    let widgets = &window.widgets;
    let root = &widgets.window;
    add_action(root, "frame", &input, || RenderingInput::Frame);
    add_action(root, "export-view", &input, || RenderingInput::ExportView);
    for view in View::ALL.into_iter().filter(|view| header::ready(*view)) {
        add_action(root, &format!("view-{}", view.word()), &input, move || {
            RenderingInput::ShowView(view)
        });
    }
    let sender = input.clone();
    widgets.header.connect(move |output| {
        sender.emit(
            match output {
                header::HeaderOutput::View(view) => RenderingInput::ShowView(view),
                header::HeaderOutput::Scene(index) => RenderingInput::SceneChosen(index),
            }
            .into(),
        );
    });
    let sender = input.clone();
    widgets.view_toolbar.connect_outputs(move |output| {
        sender.emit(
            match output {
                ViewToolbarOutput::Xray(on) => RenderingInput::Xray(on),
                ViewToolbarOutput::Pull(on) => RenderingInput::PullApart(on),
                ViewToolbarOutput::Play => RenderingInput::PlayExplode,
                ViewToolbarOutput::Apart(value) => RenderingInput::ExplodeSlid(value),
            }
            .into(),
        );
    });
    let camera = &widgets.camera;
    camera.fit.set_action_name(Some("win.frame"));
    let sender = input.clone();
    camera.connect_outputs(move |output| {
        sender.emit(
            match output {
                CameraOutput::Front => RenderingInput::LookFront,
                CameraOutput::Turn => RenderingInput::TurnAround,
            }
            .into(),
        );
    });

    // The style changed: the 3D view's backdrop follows. The style manager
    // outlives the window, so the handler goes in `shutdown`.
    let sender = input.clone();
    let style = adw::StyleManager::default().connect_dark_notify(move |manager| {
        sender.emit(
            RenderingInput::StyleChanged {
                dark: manager.is_dark(),
            }
            .into(),
        );
    });

    let viewport = &widgets.viewport;
    let sender = input.clone();
    viewport.connect_resized(move |width, height| {
        sender.emit(RenderingInput::Resized { width, height }.into());
    });
    // Left drag turns; a left click selects.
    let orbit_drag = gtk::GestureDrag::builder()
        .button(gdk::BUTTON_PRIMARY)
        .build();
    let sender = input.clone();
    orbit_drag.connect_drag_begin(move |_, _, _| {
        sender.emit(RenderingInput::DragBegun { grab_focus: true }.into());
    });
    let sender = input.clone();
    orbit_drag.connect_drag_update(move |_, x, y| {
        sender.emit(RenderingInput::Orbited { x, y }.into());
    });
    let sender = input.clone();
    orbit_drag.connect_drag_end(move |gesture, x, y| {
        if x.hypot(y) < CLICK_SLOP
            && let Some((start_x, start_y)) = gesture.start_point()
        {
            sender.emit(
                RenderingInput::Picked {
                    x: start_x + x,
                    y: start_y + y,
                }
                .into(),
            );
        }
    });
    viewport.add_controller(orbit_drag);
    // Right or middle drag moves the view sideways.
    for button in [gdk::BUTTON_SECONDARY, gdk::BUTTON_MIDDLE] {
        let pan_drag = gtk::GestureDrag::builder().button(button).build();
        let sender = input.clone();
        pan_drag.connect_drag_begin(move |_, _, _| {
            sender.emit(RenderingInput::DragBegun { grab_focus: false }.into());
        });
        let sender = input.clone();
        pan_drag.connect_drag_update(move |_, x, y| {
            sender.emit(RenderingInput::Panned { x, y }.into());
        });
        viewport.add_controller(pan_drag);
    }
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let sender = input.clone();
    scroll.connect_scroll(move |_, _, dy| {
        sender.emit(RenderingInput::Zoomed(dy).into());
        glib::Propagation::Stop
    });
    viewport.add_controller(scroll);

    // Escape, F and Delete anywhere in the window but a text field.
    let keys = gtk::EventControllerKey::new();
    let sender = input.clone();
    keys.connect_key_pressed(move |controller, key, _, modifiers| {
        let Some(root) = controller
            .widget()
            .and_then(|widget| widget.downcast::<gtk::Window>().ok())
        else {
            return glib::Propagation::Proceed;
        };
        if !modifiers.is_empty()
            || gtk::prelude::GtkWindowExt::focus(&root)
                .is_some_and(|w| w.is::<gtk::Text>() || w.is::<gtk::TextView>())
        {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Escape => {
                sender.emit(SelectionInput::Escape.into());
                glib::Propagation::Stop
            }
            gdk::Key::f | gdk::Key::F => {
                sender.emit(RenderingInput::Frame.into());
                glib::Propagation::Stop
            }
            // Delete removes the selection, saying first what goes.
            gdk::Key::Delete | gdk::Key::KP_Delete => {
                sender.emit(EditingInput::RemoveSelected.into());
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    root.add_controller(keys);

    // A mouse's Back and Forward buttons (8 and 9) anywhere in the window,
    // before the widget under the pointer sees them.
    let history = gtk::GestureClick::builder()
        .button(0)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    // Going back or forward with nowhere to go does nothing (the session
    // says so), as the disabled actions did.
    let sender = input;
    history.connect_pressed(move |gesture, _, _, _| {
        let message = match gesture.current_button() {
            MOUSE_BACK => SelectionInput::Back,
            MOUSE_FORWARD => SelectionInput::Forward,
            _ => return,
        };
        gesture.set_state(gtk::EventSequenceState::Claimed);
        sender.emit(message.into());
    });
    root.add_controller(history);

    window.style_handler = Some(style);
    window.show_backdrop(adw::StyleManager::default().is_dark());
}

impl Window {
    /// Handles a [`RenderingInput`].
    pub(in crate::ui) fn update_rendering(&mut self, message: RenderingInput) {
        match message {
            RenderingInput::Frame => self.frame(),
            RenderingInput::ExportView => self.export_view(),
            RenderingInput::ExportTo(path) => self.export_to(path),
            RenderingInput::Exported { path, saved } => match saved {
                Ok(()) => self.toast(&format!("Saved the view as {}", path.display())),
                Err(failure) => self.show_export_failure(&failure),
            },
            RenderingInput::ShowView(view) => {
                if self.session.view() != view {
                    self.dispatch(Command::ShowView(view));
                }
            }
            RenderingInput::SceneChosen(index) => {
                let name = self
                    .session
                    .drawable()
                    .and_then(|(_, graph)| Some(graph.scenes.get(index)?.name.clone()));
                self.widgets.header.scene.popdown();
                if let Some(name) = name {
                    self.dispatch(Command::ShowScene(name));
                }
            }
            RenderingInput::Xray(on) => {
                self.state.xray = on;
                self.request_render();
            }
            RenderingInput::PullApart(on) => self.pull_apart(on),
            RenderingInput::PlayExplode => self.toggle_play_explode(),
            RenderingInput::ExplodeTick(now) => self.explode_tick(now),
            RenderingInput::ExplodeSlid(value) => {
                self.state.exploding = false;
                self.dispatch(Command::SetExplode(Explode::clamped(narrow(value))));
            }
            RenderingInput::LookFront => self.look_from(0.0, FRONT_PITCH),
            RenderingInput::TurnAround => {
                let (yaw, pitch) = self
                    .state
                    .orbit
                    .map_or((0.0, FRONT_PITCH), |orbit| (orbit.yaw, orbit.pitch));
                self.look_from(yaw + std::f32::consts::PI, pitch);
            }
            RenderingInput::StyleChanged { dark } => {
                self.show_backdrop(dark);
                self.request_render();
                self.render_pictures();
                self.widgets.files.restyle(dark);
            }
            RenderingInput::Resized { width, height } => self.resized(width, height),
            RenderingInput::DragBegun { grab_focus } => {
                self.state.drag_applied = (0.0, 0.0);
                if grab_focus {
                    self.widgets.viewport.grab_focus();
                }
            }
            RenderingInput::Orbited { x, y } => {
                let (dx, dy) = self.drag_step(x, y);
                self.update_orbit(|orbit| orbit.orbited(dx, dy));
            }
            RenderingInput::Panned { x, y } => {
                let (dx, dy) = self.drag_step(x, y);
                let height = crate::convert::f32_of_int(self.widgets.viewport.height());
                self.update_orbit(|orbit| orbit.panned(dx, dy, height));
            }
            RenderingInput::Picked { x, y } => self.pick(x, y),
            RenderingInput::Zoomed(dy) => {
                self.update_orbit(|orbit| orbit.zoomed(crate::convert::f32_of(-dy)));
            }
            RenderingInput::RenderNow => self.render_now(),
            RenderingInput::PollFrame(ticket) => self.poll_frame(ticket),
        }
    }

    /// Asks the view's frame clock for one tick, which sends the message
    /// `message` makes of the frame's time.
    pub(in crate::ui) fn on_next_frame(&self, message: impl Fn(i64) -> WindowInput + 'static) {
        let input = self.input();
        self.widgets.viewport.add_tick_callback(move |_, clock| {
            input.emit(message(clock.frame_time()));
            glib::ControlFlow::Break
        });
    }

    pub(in crate::ui) fn view_changed(&mut self) {
        let view = self.session.view();
        self.widgets.header.show_view(view);
        self.widgets.views.set_visible_child_name(view.word());
        self.sync_files();
        self.request_render();
    }

    /// `true` when the 3D view is on screen: the design.
    pub(in crate::ui) fn shows_3d(&self) -> bool {
        match self.session.view() {
            View::Design => true,
            View::CutList => false,
        }
    }

    pub(in crate::ui) fn aspect(state: &State) -> f32 {
        crate::render::aspect(state.size.0, state.size.1)
    }

    /// A camera that shows everything shown.
    pub(in crate::ui) fn frame_all(state: &State, shown: &Checked) -> Option<Orbit> {
        let (_, graph) = shown.drawable()?;
        let bounds = edit_view::drawn_bounds(graph, state.posed.as_ref()?)?;
        let aspect = Self::aspect(state);
        Some(match state.orbit {
            Some(orbit) => orbit.framed(bounds, aspect),
            None => Orbit::framing(bounds, aspect),
        })
    }

    /// The node drawn as selected.
    pub(in crate::ui) fn selected_node(&self) -> Option<NodeId> {
        let session = &self.session;
        let item = session.selection()?.item()?;
        let (_, graph) = session.drawable()?;
        graph.nodes_of(item).into_iter().next()
    }

    /// Every node to draw selected: the selected item's, or those of every
    /// part and piece of hardware inside a selected assembly.
    pub(in crate::ui) fn selected_nodes(&self) -> Vec<NodeId> {
        let session = &self.session;
        let Some((design, graph)) = session.drawable() else {
            return Vec::new();
        };
        let items = session
            .selection()
            .map_or_else(Vec::new, |selection| inspect::items_in(design, selection));
        items
            .into_iter()
            .flat_map(|item| graph.nodes_of(item).into_iter().take(1))
            .collect()
    }

    /// The nodes of what the pointer is over in the Parts panel.
    pub(in crate::ui) fn hovered_nodes(&self) -> Vec<NodeId> {
        let session = &self.session;
        let state = &self.state;
        let Some((_, graph)) = session.drawable() else {
            return Vec::new();
        };
        state
            .hovered
            .iter()
            .flat_map(|item| graph.nodes_of(*item).into_iter().take(1))
            .collect()
    }

    /// F: frames the selection (every part of a selected assembly), or
    /// everything.
    pub(in crate::ui) fn frame(&mut self) {
        let nodes = self.selected_nodes();
        let Some(shown) = self.session.shown() else {
            return;
        };
        let state = &mut self.state;
        let aspect = Self::aspect(state);
        let selected = (!nodes.is_empty())
            .then(|| {
                let (_, graph) = shown.drawable()?;
                let bounds = graph.bounds_of(state.posed.as_ref()?, &nodes)?;
                let framed = state.orbit?.framed(bounds, aspect);
                Some(Orbit {
                    distance: framed.distance * FRAME_ROOM,
                    ..framed
                })
            })
            .flatten();
        state.orbit = selected.or_else(|| Self::frame_all(state, shown));
        self.request_render();
    }

    /// Choosing something frames it at once: a part, a piece of hardware
    /// or an assembly, or everything again when nothing is chosen. A
    /// problem frames itself where it is posed; focus mode frames its
    /// joint; a place in the text leaves the camera alone.
    pub(in crate::ui) fn frame_selection(&mut self) {
        let session = &self.session;
        let reframes = session.focus().is_none()
            && matches!(
                session.selection(),
                None | Some(Selection::Part(_) | Selection::Hardware(_) | Selection::Assembly(_))
            );
        if reframes {
            self.frame();
        }
    }

    /// Moves the camera where the command line asked, once the view has a
    /// size and the asked-for selection is made.
    pub(in crate::ui) fn apply_start_camera(&mut self) {
        let state = &self.state;
        if state.size.0 <= 1 || state.wanted.is_some() || state.start_camera.is_none() {
            return;
        }
        let node = self.selected_node();
        let state = &mut self.state;
        let Some(camera) = state.start_camera.take() else {
            return;
        };
        let aspect = Self::aspect(state);
        let Some(orbit) = state.orbit else {
            return;
        };
        let turned = Orbit {
            yaw: (orbit.yaw + camera.turn.to_radians()).rem_euclid(std::f32::consts::TAU),
            pitch: camera.pitch.map_or(orbit.pitch, f32::to_radians),
            ..orbit
        };
        let posed = state.posed.as_ref();
        let bounds = self.session.drawable().and_then(|(_, graph)| {
            let posed = posed?;
            match node.filter(|_| camera.frame_selection) {
                Some(id) => graph.bounds_of(posed, &[id]),
                None => edit_view::drawn_bounds(graph, posed),
            }
        });
        let framed = bounds.map_or(turned, |bounds| turned.framed(bounds, aspect));
        state.orbit = Some(Orbit {
            distance: framed.distance / camera.zoom.unwrap_or(1.0),
            ..framed
        });
        self.request_render();
    }

    /// The 3D view's area behind its cards takes the stage colour, so a
    /// page laid out around the view (hardware focus) matches it.
    pub(in crate::ui) fn show_backdrop(&self, dark: bool) {
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

    /// Turns the camera to look from a direction, framing everything.
    pub(in crate::ui) fn look_from(&mut self, yaw: f32, pitch: f32) {
        let state = &mut self.state;
        let aspect = Self::aspect(state);
        let bounds = self
            .session
            .drawable()
            .and_then(|(_, graph)| edit_view::drawn_bounds(graph, state.posed.as_ref()?));
        state.orbit = state.orbit.map(|orbit| {
            let turned = Orbit {
                yaw: yaw.rem_euclid(std::f32::consts::TAU),
                pitch,
                ..orbit
            };
            bounds.map_or(turned, |bounds| turned.framed(bounds, aspect))
        });
        self.request_render();
    }

    /// "Pull apart" switched on (all the way apart) or off (together).
    pub(in crate::ui) fn pull_apart(&mut self, on: bool) {
        // The window's own setting of the toggle is blocked, and arrives
        // as no change if it was queued before: only people's change acts.
        if self.state.pull_apart == on {
            return;
        }
        self.state.pull_apart = on;
        self.state.exploding = false;
        let apart = if on { 1.0 } else { 0.0 };
        self.dispatch(Command::SetExplode(Explode::clamped(apart)));
        self.show_view_toolbar();
    }

    /// Shows the view toolbar as the session and the window have it.
    pub(in crate::ui) fn show_view_toolbar(&mut self) {
        let explode = self.session.explode().get();
        let state = &mut self.state;
        state.pull_apart |= explode > 0.0;
        let (on, playing) = (state.pull_apart, state.exploding);
        // The toolbar never calls back for the window's own change.
        self.widgets.view_toolbar.show(on, explode, playing);
    }

    /// The pull apart play button: plays from together to apart (from the
    /// start when it is apart already), or pauses.
    pub(in crate::ui) fn toggle_play_explode(&mut self) {
        self.state.exploding = !self.state.exploding;
        let playing = self.state.exploding;
        if playing && self.session.explode().get() >= 1.0 {
            self.dispatch(Command::SetExplode(Explode::clamped(0.0)));
        }
        self.show_view_toolbar();
        if !playing {
            return;
        }
        self.state.explode_clock.last = None;
        if !self.state.explode_clock.ticking {
            self.state.explode_clock.ticking = true;
            self.on_next_frame(|now| RenderingInput::ExplodeTick(now).into());
        }
    }

    /// A frame while pulling apart plays: it moves on, and the next frame
    /// is asked for until it is all the way apart or paused.
    pub(in crate::ui) fn explode_tick(&mut self, now: i64) {
        self.state.explode_clock.ticking = false;
        if !self.state.exploding {
            return;
        }
        let seconds = self.state.explode_clock.seconds_to(now);
        let explode = self.session.explode().get();
        let next = (explode + narrow(seconds * EXPLODE_PER_SECOND)).min(1.0);
        let done = next >= 1.0;
        if done {
            self.state.exploding = false;
        }
        self.dispatch(Command::SetExplode(Explode::clamped(next)));
        if done {
            self.show_view_toolbar();
        } else {
            self.state.explode_clock.ticking = true;
            self.on_next_frame(|now| RenderingInput::ExplodeTick(now).into());
        }
    }

    /// Saves the view as posed as a glTF file, asking where.
    pub(in crate::ui) fn export_view(&mut self) {
        let name = stave_app::export_view::default_file_name(
            self.session.drawable().map(|(design, _)| design),
        );
        let dialog = gtk::FileDialog::builder()
            .title("Export View as glTF")
            .initial_name(name)
            .modal(true)
            .build();
        let input = self.input();
        dialog.save(
            Some(&self.widgets.window),
            gio::Cancellable::NONE,
            move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                input.emit(RenderingInput::ExportTo(path).into());
            },
        );
    }

    /// Saves the view as posed now into `path`, off the main thread.
    pub(in crate::ui) fn export_to(&mut self, path: PathBuf) {
        // The view as posed now, copied so that making and writing the
        // file happen off the main thread.
        let view = self
            .session
            .drawable()
            .zip(self.state.posed.as_ref())
            .map(|((_, graph), posed)| (graph.clone(), posed.clone()));
        let Some((graph, posed)) = view else {
            self.show_export_failure(&stave_app::save_failure::SaveFailure {
                heading: "Couldn't Export the View".to_owned(),
                body: "There is nothing in the 3D view to save yet. Wait for the design to show, then try again.".to_owned(),
            });
            return;
        };
        let exporting = adw::Toast::builder()
            .title("Exporting the view…")
            .timeout(0)
            .build();
        self.widgets.toasts.add_toast(exporting.clone());
        let input = self.input();
        glib::spawn_future_local(async move {
            let target = path.clone();
            let saved = gio::spawn_blocking(move || {
                stave_app::export_view::save_glb(&graph, &posed, &target)
                    .map_err(|error| stave_app::save_failure::describe_export(&error, &target))
            })
            .await
            .unwrap_or_else(|_| {
                Err(stave_app::save_failure::SaveFailure {
                    heading: "Couldn't Export the View".to_owned(),
                    body: "Stave stopped part way through making the file. Nothing was saved; try again.".to_owned(),
                })
            });
            exporting.dismiss();
            // The window may have gone; then nobody needs to know.
            let _ = input.send(RenderingInput::Exported { path, saved }.into());
        });
    }

    /// Shows a failed export of the view. It stays until it is answered,
    /// and offers another place to save to.
    pub(in crate::ui) fn show_export_failure(
        &mut self,
        failure: &stave_app::save_failure::SaveFailure,
    ) {
        let alert = adw::AlertDialog::builder()
            .heading(&failure.heading)
            .body(&failure.body)
            .close_response("cancel")
            .default_response("again")
            .build();
        alert.add_responses(&[("cancel", "_Cancel"), ("again", "Save _Elsewhere…")]);
        alert.set_response_appearance("again", adw::ResponseAppearance::Suggested);
        let input = self.input();
        alert.connect_response(None, move |_, response| {
            if response == "again" {
                input.emit(RenderingInput::ExportView.into());
            }
        });
        alert.present(Some(&self.widgets.window));
    }

    pub(in crate::ui) fn explode_changed(&mut self) {
        self.show_view_toolbar();
        let explode = self.session.explode().get();
        let quiet = self.state.focus.is_none()
            && self.state.problem.is_none()
            && self.session.selection().is_none();
        if quiet {
            self.show_hint(&if explode > 0.0 {
                format!("Pulled apart · {}%", view_tools::percent(explode))
            } else {
                String::new()
            });
        }
        self.repose();
        self.request_render();
    }

    pub(in crate::ui) fn scene_changed(&mut self) {
        let titles: Vec<String> = self
            .session
            .drawable()
            .map(|(_, graph)| {
                graph
                    .scenes
                    .iter()
                    .map(|scene| scene.title.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.widgets
            .header
            .show_scenes(&titles, self.session.scene_index());
        self.state.problem = None;
        self.show_problem_controls(None);
        self.repose();
        self.request_render();
    }

    /// Poses the design for what the window shows: focus mode, a problem
    /// at its contact, or the current scene pulled apart. In focus mode,
    /// returns its step and status for the panel.
    pub(in crate::ui) fn repose(&mut self) -> Option<(Option<String>, String)> {
        let session = &self.session;
        let (_, graph) = session.drawable()?;
        let entry = session.scene_entry()?;
        let explode = session.explode().get();
        let state = &mut self.state;
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

    pub(in crate::ui) fn pick(&mut self, x: f64, y: f64) {
        if self.edit_click(x, y) {
            return;
        }
        let viewport = &self.widgets.viewport;
        let hit = self
            .session
            .drawable()
            .zip(self.state.orbit)
            .zip(self.state.posed.as_ref())
            .and_then(|(((_, graph), orbit), posed)| {
                let ray = orbit.ray(
                    crate::convert::f32_of(x),
                    crate::convert::f32_of(y),
                    crate::convert::f32_of_int(viewport.width()),
                    crate::convert::f32_of_int(viewport.height()),
                );
                graph.pick(posed, ray)
            });
        let selection = hit.map(|hit| Selection::of_item(hit.item));
        self.dispatch(Command::Select(selection));
    }

    pub(in crate::ui) fn update_orbit(&mut self, change: impl FnOnce(Orbit) -> Orbit) {
        self.state.orbit = self.state.orbit.map(change);
        self.request_render();
    }

    /// The stage colour for the current style.
    pub(in crate::ui) fn backdrop(&self) -> Backdrop {
        if adw::StyleManager::default().is_dark() {
            Backdrop::Dark
        } else {
            Backdrop::Light
        }
    }

    /// Draws on the next frame; several requests before then draw once.
    pub(in crate::ui) fn request_render(&mut self) {
        // The Model hidden behind an open file's tab draws nothing, so a
        // screenshot asked for is taken without it.
        if self.widgets.window.is_mapped() && !self.widgets.viewport.is_mapped() {
            self.capture_if_asked();
        }
        if self.state.render_queued {
            return;
        }
        self.state.render_queued = true;
        // Drawn on the view's next frame, so several requests draw once.
        self.on_next_frame(|_| RenderingInput::RenderNow.into());
    }

    /// How each node looks outside focus mode. Choosing something only
    /// outlines it: the rest stays solid, so clicking around never switches
    /// the view between solid and see-through. With X-ray on everything
    /// but the selection is see-through; a posed problem fades what it
    /// doesn't involve. `None` draws every node as usual.
    pub(in crate::ui) fn looks(&self, selection: &[NodeId]) -> Option<Vec<Look>> {
        let session = &self.session;
        let state = &self.state;
        let (_, graph) = session.drawable()?;
        let count = graph.node_ids().count();
        let kept: &[NodeId] = match state.problem.as_ref() {
            Some(problem) => &problem.highlight.nodes,
            None if state.xray => selection,
            None => return None,
        };
        let rest = if state.xray { Look::XRay } else { Look::Dim };
        let mut looks = vec![rest; count];
        for node in kept {
            if let Some(look) = looks.get_mut(node.index()) {
                *look = Look::Normal;
            }
        }
        Some(looks)
    }

    pub(in crate::ui) fn render_now(&mut self) {
        let mut selection = self.selected_nodes();
        // What a partner link in the Parts panel points at is outlined
        // too, while the pointer is over it.
        selection.extend(self.hovered_nodes());
        // And what a name link in the Inspector or the Edit panel points at.
        selection.extend(self.link_hover_nodes());
        let backdrop = self.backdrop();
        let looks = if self.state.focus.is_some() {
            None
        } else {
            self.looks(&selection)
        };
        let ghost = self.render_ghost(backdrop);
        let rendered = if let Some(rendered) = ghost {
            rendered
        } else {
            self.state.render_queued = false;
            let session = &self.session;
            let (width, height) = self.state.size;
            // Borrowed field by field, so the renderer can be borrowed
            // mutably beside them once the marks are worked out.
            let (Some((_, graph)), Some(orbit), Some(posed)) = (
                session.drawable(),
                self.state.orbit,
                self.state.posed.as_ref(),
            ) else {
                return;
            };
            let problem = &self.state.problem;
            let focus = self.state.focus.as_ref();
            // Outside focus mode, a selected joint member shows through
            // the parts it sits in.
            let outside = if focus.is_none() {
                session.looks()
            } else {
                None
            };
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
                self.edit_marks(session, graph, posed, &orbit)
            } else {
                edit_view::EditMarks::default()
            };
            let looks = if marks.covering.is_empty() {
                looks
            } else {
                let mut looks =
                    looks.unwrap_or_else(|| vec![Look::Normal; graph.node_ids().count()]);
                for node in &marks.covering {
                    if let Some(look) = looks.get_mut(node.index()) {
                        *look = Look::XRay;
                    }
                }
                Some(looks)
            };
            let Ok(renderer) = &mut self.state.renderer else {
                return;
            };
            renderer.submit(&Frame {
                scene: graph,
                posed,
                orbit,
                selection: if focus.is_some() { &[] } else { &selection },
                highlight,
                section: plan
                    .map(|p| p.section)
                    .or_else(|| session.scene_entry().and_then(|entry| entry.section)),
                looks: focus
                    .map(|f| f.looks.as_slice())
                    .or(outside.as_deref())
                    .or(looks.as_deref()),
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
                self.show_status("Can't draw in 3D", &message);
                return;
            }
        };
        // The frame is shown once the GPU has it ready, on a later tick:
        // the main loop never waits for the GPU. A newer frame ready first
        // is shown instead, and this ticket is done with.
        self.on_next_frame(move |_| RenderingInput::PollFrame(ticket).into());
    }

    /// Shows the frame the GPU has ready, if any. A newer frame ready
    /// first is shown instead; until `ticket`'s frame has settled the
    /// next frame asks again.
    pub(in crate::ui) fn poll_frame(&mut self, ticket: Ticket) {
        let Ok(renderer) = &mut self.state.renderer else {
            return;
        };
        let (ready, settled) = (renderer.take_ready(), renderer.settled(ticket));
        match ready {
            Ok(Some(frame)) => {
                let shown = crate::render::profile::time("gtk texture", || {
                    self.widgets.viewport.show_live(frame)
                });
                if shown {
                    crate::render::profile::frame_shown();
                }
                self.show_pins();
                self.place_ghost_verdict();
                self.place_grips();
                // Save the window once a frame of the view's real size is
                // up.
                let size = self.state.size;
                if size.0 > 1 && size == self.widgets.viewport.device_size() {
                    // An edit script runs first; the frame it asks for is
                    // the one saved.
                    if self.run_edit_script() {
                        self.request_render();
                    } else {
                        self.capture_if_asked();
                    }
                }
            }
            Ok(None) => {}
            Err(error) => {
                let message = format!("{error:#}");
                self.show_status("Can't draw in 3D", &message);
            }
        }
        if !settled {
            self.on_next_frame(move |_| RenderingInput::PollFrame(ticket).into());
        }
    }

    /// Where a set of nodes shows in the 3D view, in logical pixels from
    /// its top left (the middle of their bounds as posed); `None` when it
    /// can't be worked out (nothing drawn yet, behind the camera).
    pub(in crate::ui) fn screen_point(&self, nodes: &[NodeId]) -> Option<(f32, f32)> {
        let session = &self.session;
        let state = &self.state;
        let (_, graph) = session.drawable()?;
        let bounds = graph.bounds_of(state.posed.as_ref()?, nodes)?;
        self.project(state, bounds.centre())
    }

    /// How much of the 3D view's right side the camera column takes (with
    /// the space beside it), in logical pixels: what the editing's cards
    /// and labels keep clear of at the right.
    pub(in crate::ui) fn right_column(&self) -> i32 {
        let camera = &self.widgets.camera.root;
        if camera.is_visible() {
            camera.width().saturating_add(EDGE)
        } else {
            0
        }
    }

    /// Where a point of the scene shows in the 3D view, in logical pixels.
    pub(in crate::ui) fn project(&self, state: &State, point: glam::Vec3) -> Option<(f32, f32)> {
        let viewport = &self.widgets.viewport;
        state.orbit?.project(
            point,
            crate::convert::f32_of_int(viewport.width()),
            crate::convert::f32_of_int(viewport.height()),
        )
    }

    /// Takes every pin off the model.
    pub(in crate::ui) fn hide_pins(&mut self) {
        for pin in self.pins.pins.drain(..) {
            self.widgets.design.remove_overlay(&pin.widget);
        }
        self.pins.made_for = None;
        self.pins.edge = None;
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
    pub(in crate::ui) fn show_pins(&mut self) {
        let key = {
            let session = &self.session;
            let state = &self.state;
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
    pub(in crate::ui) fn make_pins(&mut self, key: PinsKey) {
        let found: Vec<(ProblemIndex, Mark, String, PinAnchor)> = {
            let session = &self.session;
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
        let input = self.input();
        let pins: Vec<Pin> = found
            .into_iter()
            .map(|(index, mark, message, anchor)| {
                let widget = Self::pin(&input, index, mark, &message);
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
        self.pins.pins = pins;
        self.pins.made_for = Some(key);
    }

    /// Moves each pin to where its place shows now, hiding those whose
    /// place is off screen (or under a card) and naming the first of them
    /// on the chip at the view's edge.
    pub(in crate::ui) fn place_pins(&mut self) {
        let (width, height) = (
            crate::convert::f32_of_int(self.widgets.viewport.width()),
            crate::convert::f32_of_int(self.widgets.viewport.height()),
        );
        let session = &self.session;
        let state = &self.state;
        let graph = session.drawable().map(|(_, graph)| graph);
        let pins = &self.pins;
        let mut first_off: Option<usize> = None;
        let mut off_count: usize = 0;
        for (place, pin) in pins.pins.iter().enumerate() {
            let point = match &pin.anchor {
                PinAnchor::Volume(centre) => self.project(state, *centre),
                PinAnchor::Nodes(nodes) => graph
                    .zip(state.posed.as_ref())
                    .and_then(|(graph, posed)| graph.bounds_of(posed, nodes))
                    .and_then(|bounds| self.project(state, bounds.centre())),
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
        self.pins.edge = edge;
        let Some(pin) = first_off.and_then(|place| self.pins.pins.get(place)) else {
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
    /// selects the problem (posed where it happens). Its button holds only
    /// the window's input sender.
    pub(in crate::ui) fn pin(
        input: &relm4::Sender<WindowInput>,
        index: ProblemIndex,
        mark: Mark,
        message: &str,
    ) -> gtk::Box {
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
        let input = input.clone();
        button.connect_clicked(move |_| {
            input.emit(SelectionInput::ShowProblem(index.0).into());
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

    pub(in crate::ui) fn show_status(&mut self, title: &str, description: &str) {
        self.show_empty(title, description, false);
    }

    /// Shows the 3D view's empty state instead of the view.
    pub(in crate::ui) fn show_empty(&mut self, title: &str, description: &str, offers_open: bool) {
        self.widgets.empty.show(title, description, offers_open);
        self.widgets.view_stack.set_visible_child_name("status");
        self.capture_if_asked();
    }

    /// The view has a new size, in device pixels.
    pub(in crate::ui) fn resized(&mut self, width: u32, height: u32) {
        let state = &mut self.state;
        let changed = state.size != (width, height);
        let first = state.size.0 <= 1 && width > 1;
        state.size = (width, height);
        // The first camera was framed before the view had a size.
        if first && let Some(shown) = self.session.shown() {
            state.orbit = Self::frame_all(state, shown);
        }
        // ...and anything chosen by then is framed again at the real size.
        if first && self.state.focus.is_none() {
            self.frame_selection();
        }
        if first && self.state.focus.is_some() {
            self.frame_focus();
        }
        if first {
            self.apply_start_camera();
        }
        if changed {
            self.request_render();
        }
    }

    /// The part of a drag not applied yet.
    pub(in crate::ui) fn drag_step(&mut self, x: f64, y: f64) -> (f32, f32) {
        let state = &mut self.state;
        let (done_x, done_y) = state.drag_applied;
        state.drag_applied = (x, y);
        (
            crate::convert::f32_of(x - done_x),
            crate::convert::f32_of(y - done_y),
        )
    }
}
