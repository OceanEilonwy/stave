//! Wiring the widgets' signals to the window: the header, the panels,
//! the files, the finder, the 3D view's pointer and keys, and the
//! watcher that rechecks on save.

use stave_app::i18n::{fill_in, tr};
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use stave_app::check::Watcher;
use stave_app::finder::Goal;
use stave_app::session::{
    Command, Editor, Explode, LineNumber, ProblemIndex, Selection, SourceLocation, View,
};
use stave_app::{finder as finding, link};

use super::camera::{CameraKey, ORBIT_KEY_STEP};
use super::finder::Choice;
use super::widgets::narrow;
use super::{
    CLICK_SLOP, FRONT_PITCH, MOUSE_BACK, MOUSE_FORWARD, Window, app, tab_strip, workbench,
};

impl Window {
    pub(super) fn connect_events(self: &Rc<Self>) {
        let widgets = &self.widgets;
        let window = Rc::downgrade(self);
        widgets.problems.connect_choose(move |index| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Select(Some(Selection::Problem(ProblemIndex(
                    index,
                )))));
            }
        });
        let window = Rc::downgrade(self);
        widgets.problems.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        let window = Rc::downgrade(self);
        widgets.file_list.connect_open(move |path| {
            let (Some(window), Some(line)) = (window.upgrade(), LineNumber::new(1)) else {
                return;
            };
            window.dispatch(Command::GoToSource(SourceLocation {
                path: path.display().to_string(),
                line,
            }));
        });
        let header = &widgets.header;

        let window = Rc::downgrade(self);
        header.switcher.connect_active_name_notify(move |group| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let Some(view) = group.active_name().and_then(|name| View::from_word(&name)) else {
                return;
            };
            if window.session.borrow().view() != view {
                window.dispatch(Command::ShowView(view));
            }
        });

        let window = Rc::downgrade(self);
        header.scenes.connect_row_activated(move |_, row| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let name = {
                let session = window.session.borrow();
                usize::try_from(row.index())
                    .ok()
                    .and_then(|index| Some(session.drawable()?.1.scenes.get(index)?.name.clone()))
            };
            window.widgets.header.scene.popdown();
            if let Some(name) = name {
                window.dispatch(Command::ShowScene(name));
            }
        });

        let window = Rc::downgrade(self);
        widgets.tree.connect_select(move |selection| {
            if let Some(window) = window.upgrade() {
                window.select_and_frame(Some(selection));
            }
        });
        let window = Rc::downgrade(self);
        widgets.tree.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        let window = Rc::downgrade(self);
        widgets.tree.connect_focus(move |target| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Focus(target));
            }
        });
        let window = Rc::downgrade(self);
        widgets.tree.connect_hover(move |items| {
            let Some(window) = window.upgrade() else {
                return;
            };
            // The panel may say so while the window is busy with its state.
            let changed = match window.state.try_borrow_mut() {
                Ok(mut state) if state.hovered != items => {
                    state.hovered = items;
                    true
                }
                Ok(_) | Err(_) => false,
            };
            if changed {
                window.request_render();
            }
        });
        let window = Rc::downgrade(self);
        widgets.dock.connect_move(move |assembly| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Select(Some(Selection::Assembly(assembly))));
            }
        });
        let window = Rc::downgrade(self);
        widgets.view_toolbar.xray.connect_toggled(move |button| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.state.borrow_mut().xray = button.is_active();
            window.request_render();
        });
        self.connect_finder();
        let window = Rc::downgrade(self);
        widgets.inspector.connect_focus(move |target| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Focus(target));
            }
        });
        let window = Rc::downgrade(self);
        widgets.inspector.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        // Inspector links select and light what they name (`edit_view/links.rs`).
        self.connect_links(&widgets.inspector.links);
        let files = widgets.files.clone();
        widgets
            .inspector
            .connect_unsaved(move |path| files.is_unsaved(Path::new(path)));
        let window = Rc::downgrade(self);
        widgets.motion.scale.connect_value_changed(move |scale| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.motion_moved(scale.value())
        });

        // Focus mode's panel and its controls in the 3D view's area.
        let focus_panel = &widgets.inspector.focus;
        let window = Rc::downgrade(self);
        focus_panel.done.connect_clicked(move |_| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.dispatch(Command::LeaveFocus)
        });
        let window = Rc::downgrade(self);
        focus_panel.connect_source(move |location| {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::GoToSource(location));
            }
        });
        for scrub in [&widgets.step_card.scrub, &widgets.swing_card.scrub] {
            let window = Rc::downgrade(self);
            scrub.connect_changed(move |value| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                let value = narrow(value).clamp(0.0, 1.0);
                window.update_focus(|focus| {
                    focus.progress = value;
                    focus.playing = false;
                });
            });
        }
        for play in [&widgets.step_card.play, &widgets.swing_card.play] {
            let window = Rc::downgrade(self);
            play.connect_clicked(move |_| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                window.toggle_play_focus()
            });
        }
        let window = Rc::downgrade(self);
        focus_panel.xray.connect_active_notify(move |check| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let on = check.is_active();
            window.update_focus(|focus| focus.options.xray = on);
        });
        let window = Rc::downgrade(self);
        focus_panel.tint.connect_active_notify(move |check| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let on = check.is_active();
            window.update_focus(|focus| focus.options.tint_cut_faces = on);
        });
        let window = Rc::downgrade(self);
        focus_panel.fade.connect_active_notify(move |check| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let on = check.is_active();
            window.update_focus(|focus| focus.options.fade_others = on);
        });
        let window = Rc::downgrade(self);
        widgets
            .plan_switch
            .from_above
            .connect_toggled(move |button| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                let on = button.is_active();
                window.update_focus(|focus| focus.plan = on);
            });

        // A problem's motion card.
        let window = Rc::downgrade(self);
        widgets.problem_card.scrub.connect_changed(move |value| {
            if let Some(window) = window.upgrade() {
                window.problem_moved(value);
            }
        });

        // The view toolbar: Pull apart, its play button and its slider.
        let toolbar = &widgets.view_toolbar;
        let window = Rc::downgrade(self);
        toolbar.pull.connect_toggled(move |button| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let on = button.is_active();
            window.pull_apart(on);
        });
        let window = Rc::downgrade(self);
        toolbar.play.connect_clicked(move |_| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.toggle_play_explode()
        });
        let window = Rc::downgrade(self);
        toolbar.slider.connect_changed(move |value| {
            if let Some(window) = window.upgrade() {
                window.state.borrow_mut().exploding = false;
                window.dispatch(Command::SetExplode(Explode::clamped(narrow(value))));
            }
        });

        // The camera buttons.
        let camera = &widgets.camera;
        camera.fit.set_action_name(Some("win.frame"));
        let window = Rc::downgrade(self);
        camera.front.connect_clicked(move |_| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.look_from(0.0, FRONT_PITCH)
        });
        let window = Rc::downgrade(self);
        camera.turn.connect_clicked(move |_| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let (yaw, pitch) = window
                .state
                .borrow()
                .orbit
                .map_or((0.0, FRONT_PITCH), |orbit| (orbit.yaw, orbit.pitch));
            window.look_from(yaw + std::f32::consts::PI, pitch);
        });

        // The style changed: the 3D view's backdrop follows.
        let window = Rc::downgrade(self);
        adw::StyleManager::default().connect_dark_notify(move |manager| {
            if let Some(window) = window.upgrade() {
                window.show_backdrop(manager.is_dark());
                window.request_render();
                window.render_pictures();
                window
                    .widgets
                    .files
                    .restyle(adw::StyleManager::default().is_dark());
            }
        });
        self.show_backdrop(adw::StyleManager::default().is_dark());

        self.connect_files();
        self.connect_viewport();
    }

    /// The editor tells the window where the cursor is, where to go,
    /// when typing has paused and when a tab on the rail was chosen.
    pub(super) fn connect_files(self: &Rc<Self>) {
        let files = &self.widgets.files;
        let window = Rc::downgrade(self);
        files.connect_link(move |found| {
            if let Some(window) = window.upgrade() {
                window.text_linked(found);
            }
        });
        let window = Rc::downgrade(self);
        files.connect_go(move |from, to| {
            let Some(window) = window.upgrade() else {
                return;
            };
            // Where the cursor was becomes the place Back returns to.
            if let Some(from) = from {
                window.dispatch(Command::Link(Some(Selection::Location(from))));
            }
            window.dispatch(Command::GoToSource(to));
        });
        let window = Rc::downgrade(self);
        files.connect_check(move || {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::Check);
            }
        });
        let window = Rc::downgrade(self);
        files.connect_show_in_3d(move |analysis, symbol| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let selection = {
                let session = window.session.borrow();
                session.drawable().and_then(|(design, _)| {
                    link::selection_for(design, &analysis, symbol, session.selection())
                })
            };
            match selection {
                Some(selection) => window.select_and_frame(Some(selection)),
                None => {
                    // Name the thing, and say why: the view shows the last
                    // version of the design that had no errors.
                    let name = analysis
                        .symbol(symbol)
                        .map(|found| found.name.clone());
                    window.widgets.toasts.add_toast(
                        adw::Toast::builder()
                            .title(match name {
                                Some(name) => fill_in(
                                    &tr("{name} isn't in the 3D view, which shows the last version without errors"),
                                    &[("name", &name)],
                                ),
                                None => tr("That isn't in the 3D view, which shows the last version without errors"),
                            })
                            .timeout(4)
                            .build(),
                    );
                }
            }
        });
        let window = Rc::downgrade(self);
        files.connect_open(move || {
            if let Some(window) = window.upgrade() {
                window.dispatch(Command::ShowEditor(Editor::Open));
            }
        });
        // The open files as tabs beside the Model, wherever the Code
        // panel is docked.
        if let Some(code) = self.widgets.workbench.panel(workbench::PanelId::Code) {
            let page = Rc::downgrade(files);
            tab_strip::set_page_tabs(code, move || {
                page.upgrade()
                    .map(|page| page.page_tabs())
                    .unwrap_or_default()
            });
        }
        let window = Rc::downgrade(self);
        files.connect_notice(move |text| {
            if let Some(window) = window.upgrade() {
                window
                    .widgets
                    .toasts
                    .add_toast(adw::Toast::builder().title(text).timeout(4).build());
            }
        });
        let window = Rc::downgrade(self);
        self.widgets.workbench.connect_save_failed(move |text| {
            if let Some(window) = window.upgrade() {
                window
                    .widgets
                    .toasts
                    .add_toast(adw::Toast::builder().title(text).timeout(5).build());
            }
        });
        // Closing the window asks about unsaved changes first.
        let window = Rc::downgrade(self);
        self.widgets.window.connect_close_request(move |_| {
            let Some(window) = window.upgrade() else {
                return glib::Propagation::Proceed;
            };
            window.shell.remember_window(&window.widgets.window);
            window.widgets.workbench.save();
            if !window.widgets.files.has_unsaved() {
                return glib::Propagation::Proceed;
            }
            let closing = window.clone();
            window.ask_about_unsaved(move || {
                closing.widgets.files.forget_unsaved();
                closing.widgets.window.close();
            });
            glib::Propagation::Stop
        });
    }

    /// The finder finds through `stave_app::finder` and goes where a hit
    /// says: a part is selected (and framed), a problem posed, a place in
    /// the text opened in the editor.
    pub(super) fn connect_finder(self: &Rc<Self>) {
        let window = Rc::downgrade(self);
        self.widgets.finder.connect_search(move |query| {
            let Some(window) = window.upgrade() else {
                return Vec::new();
            };
            let analysis = window.widgets.files.analysis();
            let session = window.session.borrow();
            finding::search(
                finding::Sources {
                    shown: session.shown().map(|c| &**c),
                    latest: session.latest().map(|c| &**c),
                    analysis: analysis.as_deref(),
                },
                query,
            )
        });
        let window = Rc::downgrade(self);
        self.widgets.finder.connect_hit(move |hit, choice| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match (hit.goal, choice) {
                // Resting on a part shows it, without going there.
                (
                    Goal::Select(
                        selection @ (Selection::Part(_)
                        | Selection::Hardware(_)
                        | Selection::Assembly(_)),
                    ),
                    Choice::Preview,
                ) => {
                    window.state.borrow_mut().linking = true;
                    window.dispatch(Command::Link(Some(selection)));
                    window.state.borrow_mut().linking = false;
                    window.frame();
                }
                (Goal::Select(_) | Goal::Open(_), Choice::Preview) => {}
                (Goal::Select(selection), Choice::Go) => {
                    window.select_and_frame(Some(selection));
                }
                (Goal::Select(selection), Choice::OpenText) => {
                    let placed = {
                        let session = window.session.borrow();
                        session
                            .shown()
                            .and_then(|shown| link::placed_at(shown, &selection))
                    };
                    window.select_and_frame(Some(selection));
                    if let Some(placed) = placed {
                        window.dispatch(Command::GoToSource(placed));
                    }
                }
                (Goal::Open(location), Choice::Go | Choice::OpenText) => {
                    window.dispatch(Command::GoToSource(location));
                }
            }
        });
    }

    pub(super) fn connect_viewport(self: &Rc<Self>) {
        let window = Rc::downgrade(self);
        self.widgets.pictures.connect_resized(move || {
            if let Some(window) = window.upgrade() {
                window.schedule_pictures();
            }
        });
        let viewport = &self.widgets.viewport;
        let window = Rc::downgrade(self);
        viewport.connect_resized(move |width, height| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let changed = {
                let mut state = window.state.borrow_mut();
                let changed = state.size != (width, height);
                let first = state.size.0 <= 1 && width > 1;
                state.size = (width, height);
                // The first camera was framed before the view had a size.
                if first && let Some(shown) = window.session.borrow().shown() {
                    state.orbit = Self::frame_all(&state, shown);
                }
                (changed, first)
            };
            let (changed, first) = changed;
            // ...and anything chosen by then is framed again at the real
            // size.
            if first && window.state.borrow().focus.is_none() {
                window.frame_selection();
            }
            if first && window.state.borrow().focus.is_some() {
                window.frame_focus();
            }
            if first {
                window.apply_start_camera();
            }
            if changed {
                window.request_render();
            }
        });

        // Left drag turns; a left click selects.
        let orbit_drag = gtk::GestureDrag::builder()
            .button(gdk::BUTTON_PRIMARY)
            .build();
        let window = Rc::downgrade(self);
        orbit_drag.connect_drag_begin(move |_, _, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            window.state.borrow_mut().drag_applied = (0.0, 0.0);
            window.widgets.viewport.grab_focus();
        });
        let window = Rc::downgrade(self);
        orbit_drag.connect_drag_update(move |_, x, y| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let (dx, dy) = window.drag_step(x, y);
            window.update_orbit(|orbit| orbit.orbited(dx, dy));
        });
        let window = Rc::downgrade(self);
        orbit_drag.connect_drag_end(move |gesture, x, y| {
            let Some(window) = window.upgrade() else {
                return;
            };
            if x.hypot(y) < CLICK_SLOP
                && let Some((start_x, start_y)) = gesture.start_point()
            {
                window.pick(start_x + x, start_y + y);
            }
        });
        viewport.add_controller(orbit_drag);

        // Right or middle drag moves the view sideways.
        for button in [gdk::BUTTON_SECONDARY, gdk::BUTTON_MIDDLE] {
            let pan_drag = gtk::GestureDrag::builder().button(button).build();
            let window = Rc::downgrade(self);
            pan_drag.connect_drag_begin(move |_, _, _| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                window.state.borrow_mut().drag_applied = (0.0, 0.0);
            });
            let window = Rc::downgrade(self);
            pan_drag.connect_drag_update(move |_, x, y| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                let (dx, dy) = window.drag_step(x, y);
                let height = crate::convert::f32_of_int(window.widgets.viewport.height());
                window.update_orbit(|orbit| orbit.panned(dx, dy, height));
            });
            viewport.add_controller(pan_drag);
        }

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let window = Rc::downgrade(self);
        scroll.connect_scroll(move |_, _, dy| {
            let Some(window) = window.upgrade() else {
                return glib::Propagation::Proceed;
            };
            window.update_orbit(|orbit| orbit.zoomed(crate::convert::f32_of(-dy)));
            glib::Propagation::Stop
        });
        viewport.add_controller(scroll);

        // While the view has the keyboard, the arrow keys turn it and
        // plus and minus zoom (`view.*` actions, in the shortcuts dialog).
        let actions = gio::SimpleActionGroup::new();
        let camera_keys: [(&str, CameraKey); 6] = [
            ("turn-left", CameraKey::Turn(-ORBIT_KEY_STEP, 0.0)),
            ("turn-right", CameraKey::Turn(ORBIT_KEY_STEP, 0.0)),
            ("look-higher", CameraKey::Turn(0.0, ORBIT_KEY_STEP)),
            ("look-lower", CameraKey::Turn(0.0, -ORBIT_KEY_STEP)),
            ("zoom-in", CameraKey::Zoom(1.0)),
            ("zoom-out", CameraKey::Zoom(-1.0)),
        ];
        for (name, key) in camera_keys {
            let action = gio::SimpleAction::new(name, None);
            let window = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(window) = window.upgrade() {
                    window.update_orbit(|orbit| key.apply(orbit));
                }
            });
            actions.add_action(&action);
        }
        viewport.insert_action_group("view", Some(&actions));
        let view_keys = app::shortcut_controller(app::KeyScope::View);
        view_keys.set_scope(gtk::ShortcutScope::Local);
        viewport.add_controller(view_keys);

        // F (frame) and Escape anywhere in the window but a text field,
        // after the focused widget had its turn.
        let shortcuts = app::shortcut_controller(app::KeyScope::Window);
        shortcuts.set_scope(gtk::ShortcutScope::Local);
        self.widgets.window.add_controller(shortcuts);

        // Delete anywhere in the window but a text field.
        let keys = gtk::EventControllerKey::new();
        let window = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(window) = window.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !modifiers.is_empty()
                || gtk::prelude::GtkWindowExt::focus(&window.widgets.window)
                    .is_some_and(|w| w.is::<gtk::Text>() || w.is::<gtk::TextView>())
            {
                return glib::Propagation::Proceed;
            }
            match key {
                // Delete removes the selection, saying first what goes.
                gdk::Key::Delete | gdk::Key::KP_Delete if window.remove_selected() => {
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.widgets.window.add_controller(keys);

        // A mouse's Back and Forward buttons (8 and 9) anywhere in the
        // window, before the widget under the pointer sees them.
        let history = gtk::GestureClick::builder()
            .button(0)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let window = Rc::downgrade(self);
        history.connect_pressed(move |gesture, _, _, _| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let action = match gesture.current_button() {
                MOUSE_BACK => "win.back",
                MOUSE_FORWARD => "win.forward",
                _ => return,
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let _ = WidgetExt::activate_action(&window.widgets.window, action, None);
        });
        self.widgets.window.add_controller(history);
    }

    /// Watches the project folder and checks again after a `.stave` file
    /// is saved (once the folder is quiet).
    pub(super) fn watch(self: &Rc<Self>) {
        let folder = self.session.borrow().folder().to_path_buf();
        let (sender, receiver) = async_channel::unbounded::<()>();
        let watcher = Watcher::start(&folder, move || {
            // The window may be gone; then nobody needs to know.
            let _ = sender.send_blocking(());
        });
        match watcher {
            Ok(watcher) => {
                self.watcher.replace(Some(watcher));
            }
            Err(error) => {
                self.watcher.replace(None);
                eprintln!("warning: {error}");
                self.widgets.toasts.add_toast(
                    adw::Toast::builder()
                        .title(tr(
                            "Not watching the folder for changes: check again with Ctrl+R",
                        ))
                        .timeout(6)
                        .build(),
                );
                return;
            }
        }
        let window = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while receiver.recv().await.is_ok() {
                let Some(window) = window.upgrade() else {
                    break;
                };
                window.dispatch(Command::Check);
            }
        });
    }
}
