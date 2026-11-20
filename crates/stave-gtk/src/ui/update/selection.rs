//! What is selected and focused: the selection, focus mode, a posed
//! problem and the motion sliders, the parts and problems panels, and the
//! finder's hits.

use super::add_action;
use crate::ui::chrome::{MotionBarOutput, MotionShown};
use crate::ui::dock::DockOutput;
use crate::ui::finder::FinderOutput;
use crate::ui::focus_panel::FocusPanelOutput;
use crate::ui::focus_stage::{PicturesOutput, ProblemCardOutput, StepCardOutput, SwingCardOutput};
use crate::ui::inspector::InspectorOutput;
use crate::ui::panels::{FilesListOutput, ProblemsOutput};
use crate::ui::tree::TreeOutput;
use crate::ui::view_tools::PlanSwitchOutput;
// The window's own items and imports, as the handlers had them in mod.rs.
use crate::ui::*;

/// What the selection group is told.
#[derive(Debug)]
pub(in crate::ui) enum SelectionInput {
    /// Selects something (or nothing), without moving the camera.
    Select(Option<Selection>),
    /// Selects something and frames it.
    SelectAndFrame(Option<Selection>),
    /// Opens a place in the text.
    GoToSource(SourceLocation),
    /// Opens focus mode on a joint or piece of hardware.
    Focus(Target),
    /// Leaves focus mode.
    LeaveFocus,
    /// A partner link in the Parts panel is under the pointer: these items
    /// are outlined (none when it leaves).
    Hovered(Vec<stave_scene::Item>),
    /// Back in the history.
    Back,
    /// Forward in the history.
    Forward,
    /// The next problem.
    NextProblem,
    /// The previous problem.
    PreviousProblem,
    /// Escape: cancels an edit, leaves focus mode or clears the selection.
    Escape,
    /// Shows a problem (the chip at the view's edge, a pin on the model).
    ShowProblem(usize),
    /// Opens the finder.
    Find,
    /// The finder asks what a query finds.
    FinderSearch(String),
    /// A finder's hit was chosen.
    FinderHit {
        /// The hit.
        hit: finding::Hit,
        /// What was asked of it.
        choice: Choice,
    },
    /// The selection's motion slider moved.
    MotionMoved(f64),
    /// A posed problem's slider moved.
    ProblemMoved(f64),
    /// Focus mode's scrub moved (0 to 1).
    FocusScrubbed(f64),
    /// Focus mode's play button.
    PlayFocus,
    /// A frame while focus mode plays, at this frame time.
    FocusTick(i64),
    /// Focus mode's X-ray check.
    FocusXray(bool),
    /// Focus mode's tint check.
    FocusTint(bool),
    /// Focus mode's fade check.
    FocusFade(bool),
    /// Focus mode's From above switch.
    FocusPlan(bool),
    /// Hardware focus's small views changed size: the size in device
    /// pixels to render them at, `None` until they have room.
    PicturesResized(Option<(u32, u32)>),
    /// The small views' size has settled (for the ask with this number).
    PicturesDue(u64),
    /// One of hardware focus's small views was chosen.
    PictureChosen(usize),
}

impl From<SelectionInput> for WindowInput {
    fn from(message: SelectionInput) -> WindowInput {
        WindowInput::Selection(message)
    }
}

/// Wires the parts, problems and Details panels, the dock's chips, focus
/// mode's controls, the motion sliders, the finder and the history's
/// actions to [`SelectionInput`]s.
pub(in crate::ui) fn wire(window: &mut Window) {
    let input = window.input();
    let widgets = &window.widgets;
    let root = &widgets.window;
    add_action(root, "back", &input, || SelectionInput::Back);
    add_action(root, "forward", &input, || SelectionInput::Forward);
    add_action(root, "next-problem", &input, || SelectionInput::NextProblem);
    add_action(root, "previous-problem", &input, || {
        SelectionInput::PreviousProblem
    });
    add_action(root, "find", &input, || SelectionInput::Find);
    add_action(root, "escape", &input, || SelectionInput::Escape);
    // The chip at the view's edge names a problem off screen.
    let off_screen = gio::SimpleAction::new("show-problem", Some(glib::VariantTy::UINT64));
    let sender = input.clone();
    off_screen.connect_activate(move |_, parameter| {
        if let Some(index) = parameter
            .and_then(|p| p.get::<u64>())
            .and_then(|index| usize::try_from(index).ok())
        {
            sender.emit(SelectionInput::ShowProblem(index).into());
        }
    });
    root.add_action(&off_screen);

    // The panels, the finder, focus mode's cards, the dock's chips, From
    // above and the motion slider send their outputs as the window's
    // messages, forwarded when `build_widgets` launched them (the `_said`
    // functions below).
}

/// The Problems panel's outputs as the window's messages.
pub(in crate::ui) fn problems_said(output: ProblemsOutput) -> WindowInput {
    match output {
        ProblemsOutput::Choose(index) => SelectionInput::ShowProblem(index),
        ProblemsOutput::Source(location) => SelectionInput::GoToSource(location),
    }
    .into()
}

/// The Files panel's outputs as the window's messages: a file opens at
/// its first line.
pub(in crate::ui) fn file_list_said(output: FilesListOutput) -> WindowInput {
    match output {
        FilesListOutput::Open(path) => SelectionInput::GoToSource(SourceLocation {
            path: path.display().to_string(),
            line: LineNumber::FIRST,
        })
        .into(),
    }
}

/// The Parts panel's outputs as the window's messages.
pub(in crate::ui) fn tree_said(output: TreeOutput) -> WindowInput {
    match output {
        TreeOutput::Select(selection) => SelectionInput::SelectAndFrame(Some(selection)),
        TreeOutput::Source(location) => SelectionInput::GoToSource(location),
        TreeOutput::Focus(target) => SelectionInput::Focus(target),
        TreeOutput::Hover(items) => SelectionInput::Hovered(items),
    }
    .into()
}

/// The inspector's outputs as the window's messages. A link it draws is
/// the editing group's: a click selects or lights what it names, the
/// pointer on it lights it.
pub(in crate::ui) fn inspector_said(output: InspectorOutput) -> WindowInput {
    match output {
        InspectorOutput::Focus(target) => SelectionInput::Focus(target).into(),
        InspectorOutput::Source(location) => SelectionInput::GoToSource(location).into(),
        InspectorOutput::Link(target) => EditingInput::LinkClicked(target).into(),
        InspectorOutput::Hover(target) => EditingInput::LinkHovered(target).into(),
    }
}

/// The finder's outputs as the window's messages. It finds through
/// `stave_app::finder`, and lists the verbs a query names through the
/// editing group; the window answers both searches from its own update.
pub(in crate::ui) fn finder_said(output: FinderOutput) -> WindowInput {
    match output {
        FinderOutput::Search(query) => SelectionInput::FinderSearch(query).into(),
        FinderOutput::Hit { hit, choice } => SelectionInput::FinderHit { hit, choice }.into(),
        FinderOutput::SearchActions(query) => EditingInput::FinderActions(query).into(),
        FinderOutput::Action(index) => EditingInput::RunFinderAction(index).into(),
    }
}

/// Focus mode's panel's outputs as the window's messages. The panel turns
/// its checks on when it shows a new focus, with their handlers blocked,
/// so these are people's alone; they are handled in the order the checks
/// changed, so the model ends as they show.
pub(in crate::ui) fn focus_panel_said(output: FocusPanelOutput) -> WindowInput {
    match output {
        FocusPanelOutput::Done => SelectionInput::LeaveFocus,
        FocusPanelOutput::Source(location) => SelectionInput::GoToSource(location),
        FocusPanelOutput::Xray(on) => SelectionInput::FocusXray(on),
        FocusPanelOutput::Tint(on) => SelectionInput::FocusTint(on),
        FocusPanelOutput::Fade(on) => SelectionInput::FocusFade(on),
    }
    .into()
}

/// The joint's step player's outputs as the window's messages.
pub(in crate::ui) fn step_card_said(output: StepCardOutput) -> WindowInput {
    match output {
        StepCardOutput::Play => SelectionInput::PlayFocus,
        StepCardOutput::Scrubbed(value) => SelectionInput::FocusScrubbed(value),
    }
    .into()
}

/// Hardware focus's swing card's outputs as the window's messages.
pub(in crate::ui) fn swing_card_said(output: SwingCardOutput) -> WindowInput {
    match output {
        SwingCardOutput::Play => SelectionInput::PlayFocus,
        SwingCardOutput::Scrubbed(value) => SelectionInput::FocusScrubbed(value),
    }
    .into()
}

/// A problem's motion card's outputs as the window's messages.
pub(in crate::ui) fn problem_card_said(output: ProblemCardOutput) -> WindowInput {
    match output {
        ProblemCardOutput::Moved(value) => SelectionInput::ProblemMoved(value).into(),
    }
}

/// Hardware focus's small views' outputs as the window's messages.
pub(in crate::ui) fn pictures_said(output: PicturesOutput) -> WindowInput {
    match output {
        PicturesOutput::Resized(size) => SelectionInput::PicturesResized(size),
        PicturesOutput::Chosen(index) => SelectionInput::PictureChosen(index),
    }
    .into()
}

/// The motion bar's outputs as the window's messages (forwarded at
/// launch). It is set with its handler blocked, so they are people's.
pub(in crate::ui) fn motion_said(output: MotionBarOutput) -> WindowInput {
    match output {
        MotionBarOutput::Moved(value) => SelectionInput::MotionMoved(value).into(),
    }
}

/// The plan switch's outputs as the window's messages.
pub(in crate::ui) fn plan_switch_said(output: PlanSwitchOutput) -> WindowInput {
    match output {
        PlanSwitchOutput::FromAbove(plan) => SelectionInput::FocusPlan(plan).into(),
    }
}

/// The dock's outputs as the window's messages: a chip selects its
/// assembly.
pub(in crate::ui) fn dock_said(output: DockOutput) -> WindowInput {
    match output {
        DockOutput::Move(assembly) => {
            SelectionInput::Select(Some(Selection::Assembly(assembly))).into()
        }
    }
}

impl Window {
    /// Handles a [`SelectionInput`].
    pub(in crate::ui) fn update_selection(&mut self, message: SelectionInput) {
        match message {
            SelectionInput::Select(selection) => self.dispatch(Command::Select(selection)),
            SelectionInput::SelectAndFrame(selection) => self.select_and_frame(selection),
            SelectionInput::GoToSource(location) => self.dispatch(Command::GoToSource(location)),
            SelectionInput::Focus(target) => self.dispatch(Command::Focus(target)),
            SelectionInput::LeaveFocus => self.dispatch(Command::LeaveFocus),
            SelectionInput::Hovered(items) => {
                if self.state.hovered != items {
                    self.state.hovered = items;
                    self.request_render();
                }
            }
            SelectionInput::Back => self.dispatch(Command::Back),
            SelectionInput::Forward => self.dispatch(Command::Forward),
            SelectionInput::NextProblem => self.dispatch(Command::NextProblem),
            SelectionInput::PreviousProblem => self.dispatch(Command::PreviousProblem),
            SelectionInput::Escape => {
                if self.cancel_edit() {
                    return;
                }
                if self.session.focus().is_some() {
                    self.dispatch(Command::LeaveFocus);
                } else {
                    self.dispatch(Command::Select(None));
                }
            }
            SelectionInput::ShowProblem(index) => {
                self.dispatch(Command::Select(Some(Selection::Problem(ProblemIndex(
                    index,
                )))));
            }
            SelectionInput::Find => self.widgets.finder.present(&self.widgets.window, ""),
            SelectionInput::FinderSearch(query) => self.finder_search(&query),
            SelectionInput::FinderHit { hit, choice } => self.finder_hit(hit, choice),
            SelectionInput::MotionMoved(value) => self.motion_moved(value),
            SelectionInput::ProblemMoved(value) => self.problem_moved(value),
            SelectionInput::FocusScrubbed(value) => {
                let value = narrow(value).clamp(0.0, 1.0);
                self.update_focus(|focus| {
                    focus.progress = value;
                    focus.playing = false;
                });
            }
            SelectionInput::PlayFocus => self.toggle_play_focus(),
            SelectionInput::FocusTick(now) => self.focus_tick(now),
            SelectionInput::FocusXray(on) => self.switch_focus(on, |focus| &mut focus.options.xray),
            SelectionInput::FocusTint(on) => {
                self.switch_focus(on, |focus| &mut focus.options.tint_cut_faces);
            }
            SelectionInput::FocusFade(on) => {
                self.switch_focus(on, |focus| &mut focus.options.fade_others);
            }
            SelectionInput::FocusPlan(on) => self.switch_focus(on, |focus| &mut focus.plan),
            SelectionInput::PicturesResized(size) => {
                self.widgets.pictures.keep_size(size);
                if size.is_some() {
                    self.schedule_pictures();
                }
            }
            SelectionInput::PicturesDue(asked) => {
                // Only the last ask renders.
                if asked == self.state.pictures_asked {
                    self.render_pictures();
                }
            }
            SelectionInput::PictureChosen(index) => {
                let progress = self
                    .state
                    .focus
                    .as_ref()
                    .and_then(|f| f.focus.key_poses.get(index).map(|pose| pose.progress));
                if let Some(progress) = progress {
                    self.update_focus(|focus| {
                        focus.progress = progress;
                        focus.playing = false;
                    });
                }
            }
        }
    }

    /// Selects something and frames it: for the parts list, the finder,
    /// "show in 3D" and the command line. Clicking in the 3D view selects
    /// without moving the camera (`pick`). The 3D View comes to the front
    /// (brought back if it was closed), so what was chosen can be seen.
    pub(in crate::ui) fn select_and_frame(&mut self, selection: Option<Selection>) {
        self.widgets.workbench.present(workbench::PanelId::View);
        self.dispatch(Command::Select(selection));
        self.frame_selection();
    }

    pub(in crate::ui) fn selection_changed(&mut self) {
        let (selection, inspection, problem, focused) = {
            let session = &self.session;
            let selection = session.selection().cloned();
            let inspection = session
                .shown()
                .zip(selection.as_ref())
                .and_then(|(shown, selection)| inspect::inspect(shown, selection));
            let problem = session.selected_problem().cloned();
            let details = if let (Some(Selection::Problem(index)), Some(latest)) =
                (&selection, session.latest())
            {
                stave_app::problems::details(latest, *index)
            } else {
                None
            };
            (
                selection,
                inspection,
                problem.zip(details),
                session.focus().is_some(),
            )
        };
        let linked = {
            let session = &self.session;
            session
                .drawable()
                .zip(selection.as_ref())
                .and_then(|((design, _), selection)| {
                    let placed = link::placed_at(session.shown()?, selection)?;
                    link::chip(design, selection, &placed)
                })
        };
        self.widgets
            .workbench
            .status
            .set_label(linked.as_deref().unwrap_or(""));
        self.widgets.tree.show_selection(selection.as_ref());
        self.widgets.problems.show_selected(match &selection {
            Some(Selection::Problem(index)) => Some(index.0),
            Some(
                Selection::Part(_)
                | Selection::Hardware(_)
                | Selection::Joint { .. }
                | Selection::Member(_)
                | Selection::Assembly(_)
                | Selection::Location(_),
            )
            | None => None,
        });
        self.sync_files();
        if focused {
            self.show_card();
            return;
        }
        let explode = self.session.explode().get();
        self.show_hint(&if explode > 0.0 && selection.is_none() {
            fill_in(
                &tr("Pulled apart · {percent}%"),
                &[("percent", &view_tools::percent(explode))],
            )
        } else {
            String::new()
        });
        // A problem is read in the Problems panel; Details keeps to the
        // design.
        self.widgets.inspector.show(inspection.as_ref());
        // Pose a problem where it happens, or offer to open the selection.
        match problem.and_then(|(problem, details)| Some((problem.contact?, details.motion))) {
            Some((contact, card)) => self.pose_problem(contact, card),
            None => {
                self.state.problem = None;
                self.show_problem_controls(None);
                let moves_with = match &selection {
                    Some(Selection::Assembly(id)) => Some(*id),
                    Some(
                        Selection::Part(_)
                        | Selection::Hardware(_)
                        | Selection::Joint { .. }
                        | Selection::Member(_)
                        | Selection::Location(_)
                        | Selection::Problem(_),
                    )
                    | None => inspection.and_then(|i| i.moves_with),
                };
                self.update_opening(moves_with);
            }
        }
        self.show_card();
        self.show_moves();
        self.grips_follow_selection();
        self.describe_view();
        self.request_render();
    }

    /// Tells screen readers what the 3D view shows: the project, what is
    /// selected and what the last check found.
    pub(in crate::ui) fn describe_view(&self) {
        let session = &self.session;
        let selected = session.selection().and_then(|selection| match selection {
            Selection::Problem(_) => session.selected_problem().map(|problem| {
                fill_in(
                    &tr("The problem “{message}”"),
                    &[("message", &problem.message)],
                )
            }),
            Selection::Part(_)
            | Selection::Hardware(_)
            | Selection::Joint { .. }
            | Selection::Member(_)
            | Selection::Assembly(_)
            | Selection::Location(_) => session
                .shown()
                .and_then(|shown| inspect::inspect(shown, selection))
                .map(|inspection| inspection.title),
        });
        let name = fill_in(
            &tr("3D view of {project}"),
            &[("project", &session.title())],
        );
        let description =
            view_description(selected.as_deref(), &session.check_status().pill().label);
        self.widgets.viewport.describe(&name, &description);
    }

    /// The Details panel always shows the selection's account; focus mode
    /// brings it forward, since its controls are there.
    pub(in crate::ui) fn show_card(&mut self) {
        if self.session.focus().is_some() {
            self.widgets.workbench.present(workbench::PanelId::Details);
        }
        self.update_face_button();
    }

    /// A short line at the top of the 3D view ("Focused on one joint · Esc
    /// to go back"); empty hides it.
    pub(in crate::ui) fn show_hint(&self, text: &str) {
        self.widgets.hint.set_label(text);
        self.widgets.hint_box.set_visible(!text.is_empty());
    }

    /// The dock's chips: every assembly that moves, with its mark, the
    /// one the motion slider shows marked current.
    pub(in crate::ui) fn show_moves(&mut self) {
        let (moving, current, sliding) = {
            let session = &self.session;
            let state = &self.state;
            let moving: Vec<(AssemblyId, String, Option<Mark>)> = session
                .drawable()
                .map(|(design, graph)| {
                    design
                        .assemblies()
                        .iter()
                        .filter(|a| a.parent.is_none() && graph.motion_range(a.id).is_some())
                        .map(|a| (a.id, a.name.clone(), state.marks.assembly(a.id)))
                        .collect()
                })
                .unwrap_or_default();
            (
                moving,
                state.opening.map(|o| o.assembly),
                state.motion_shown,
            )
        };
        // While the selection's own slider shows, the chips step aside
        // (V2C_Spatial.dc.html): there isn't room for both.
        self.widgets
            .dock
            .show_moves(if sliding { &[] } else { &moving }, current);
    }

    /// Opens or leaves focus mode to match the session.
    pub(in crate::ui) fn focus_changed(&mut self) {
        let wanted = self.session.focus();
        let current = self.state.focus.as_ref().map(|f| f.focus.target);
        if wanted == current {
            return;
        }
        match wanted {
            Some(target) => self.open_focus(target),
            None => self.leave_focus(),
        }
    }

    pub(in crate::ui) fn open_focus(&mut self, target: Target) {
        let found = {
            let session = &self.session;
            session.shown().and_then(|shown| {
                let (design, graph) = shown.drawable()?;
                let focus = focus::open(&shown.opened, design, graph, target)?;
                let name = focus
                    .motion()
                    .and_then(|(assembly, _)| design.assembly(assembly))
                    .map(|assembly| assembly.name.clone());
                Some((focus, name))
            })
        };
        let Some((found, moving_name)) = found else {
            return;
        };
        let start = self.state.focus_start.take();
        let (start_plan, start_step) = start.unwrap_or((None, None));
        let progress = start_step.unwrap_or(if found.assembles() { 0.0 } else { 0.4 });
        let plan = found.plan().is_some() && start_plan.unwrap_or(found.prefers_plan());
        {
            let state = &mut self.state;
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
        self.show_motion(None);
        self.show_problem_controls(None);
        self.layout_for_focus(moving_name.as_deref());
        self.hide_pins();
        let texts = self.repose();
        self.frame_focus();
        {
            let state = &self.state;
            self.widgets
                .inspector
                .focus
                .show(state.focus.as_ref().map(|f| &f.focus));
        }
        self.show_focus_controls();
        self.widgets.inspector.show_focus();
        let assembles = self
            .state
            .focus
            .as_ref()
            .is_some_and(|f| f.focus.assembles());
        self.show_hint(&if assembles {
            tr("Focused on one joint · Esc to go back")
        } else {
            tr("Focused on one piece of hardware · Esc to go back")
        });
        self.show_focus_texts(texts);
        self.show_progress();
        self.render_pictures();
        self.request_render();
    }

    /// Frames the joint or the hardware (from the direction focus asks
    /// for).
    pub(in crate::ui) fn frame_focus(&mut self) {
        let state = &mut self.state;
        let aspect = Self::aspect(state);
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
    pub(in crate::ui) fn layout_for_focus(&self, moving_name: Option<&str>) {
        let state = &self.state;
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
                .show_name(moving_name.unwrap_or("it"), range.turns());
        }
        widgets.caption.root.set_visible(page);
        widgets.pictures.root.set_visible(false);
        widgets
            .plan_switch
            .root
            .set_visible(focus.is_some_and(|f| f.focus.plan().is_some()));
        widgets.camera.root.set_visible(focus.is_none());
        widgets.offer_view_tools(focus.is_none());
        self.show_caption();
    }

    /// The line over hardware focus's view: from above or in 3D.
    pub(in crate::ui) fn show_caption(&self) {
        let state = &self.state;
        let Some(focus) = state.focus.as_ref() else {
            return;
        };
        let (text, hint) = match focus.focus.plan_caption().filter(|_| focus.plan) {
            Some(caption) => (caption.to_owned(), tr("back ↑ · front ↓")),
            None => (
                fill_in(&tr("In 3D around {name}"), &[("name", &focus.focus.title)]),
                tr("drag to turn · scroll to zoom"),
            ),
        };
        self.widgets.caption.show(&text, &hint);
    }

    pub(in crate::ui) fn leave_focus(&mut self) {
        self.state.focus = None;
        self.widgets.inspector.focus.show(None);
        self.layout_for_focus(None);
        self.show_hint("");
        self.repose();
        self.selection_changed();
    }

    pub(in crate::ui) fn show_focus_texts(&self, texts: Option<(Option<String>, String)>) {
        if let Some((step, status)) = texts {
            self.widgets.step_card.show_frame(step.as_deref(), &status);
        }
    }

    /// Moves the step player's and the swing card's sliders to the
    /// progress, with the swing's reading and the small view nearest it
    /// ringed.
    pub(in crate::ui) fn show_progress(&self) {
        let state = &self.state;
        let Some(focus) = state.focus.as_ref() else {
            return;
        };
        let progress = f64::from(focus.progress);
        let widgets = &self.widgets;
        widgets.step_card.set_progress(progress);
        widgets.swing_card.set_progress(progress);
        widgets.step_card.show_playing(focus.playing);
        widgets.swing_card.show_playing(focus.playing);
        if let Some((_, range)) = focus.focus.motion() {
            widgets
                .swing_card
                .show_value(&stave_scene::focus::amount_text(
                    range,
                    focus.progress * range.full(),
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

    /// The scrub moved, or a picture was chosen: poses again.
    pub(in crate::ui) fn update_focus(&mut self, change: impl FnOnce(&mut FocusState)) {
        let Some(focus) = self.state.focus.as_mut() else {
            return;
        };
        change(focus);
        self.show_focus_change();
    }

    /// People set one of focus mode's checks or its From above switch to
    /// `on`: the option `switch` picks out follows it and focus poses
    /// again. A value the option already has changes nothing.
    fn switch_focus(&mut self, on: bool, switch: impl FnOnce(&mut FocusState) -> &mut bool) {
        let Some(focus) = self.state.focus.as_mut() else {
            return;
        };
        let option = switch(focus);
        let changed = *option != on;
        *option = on;
        // The controls show the model again: a change people made while
        // the window was setting them (focus mode opening in the same
        // turn) is shown, never left behind.
        self.show_focus_controls();
        if changed {
            self.show_focus_change();
        }
    }

    /// Shows focus mode's options on its checks and From above on its
    /// switch, as the model has them. Neither calls back.
    fn show_focus_controls(&self) {
        let Some(focus) = self.state.focus.as_ref() else {
            return;
        };
        self.widgets.inspector.focus.show_options(focus.options);
        self.widgets.plan_switch.show(focus.plan);
    }

    /// Focus changed: poses again and shows it.
    fn show_focus_change(&mut self) {
        let texts = self.repose();
        self.show_focus_texts(texts);
        self.show_progress();
        self.show_caption();
        self.request_render();
    }

    /// The play button: plays from where it is (from the start when it is
    /// at the end), or pauses.
    pub(in crate::ui) fn toggle_play_focus(&mut self) {
        let playing = {
            let state = &mut self.state;
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
        self.state.focus_clock.last = None;
        if !self.state.focus_clock.ticking {
            self.state.focus_clock.ticking = true;
            self.on_next_frame(|now| SelectionInput::FocusTick(now).into());
        }
    }

    /// A frame while focus mode plays: the progress moves on, and the
    /// next frame is asked for until it is done or paused.
    pub(in crate::ui) fn focus_tick(&mut self, now: i64) {
        self.state.focus_clock.ticking = false;
        let seconds = self.state.focus_clock.seconds_to(now);
        let done = {
            let Some(focus) = self.state.focus.as_mut() else {
                return;
            };
            if !focus.playing {
                return;
            }
            focus.progress = (focus.progress + narrow(seconds * PLAY_PER_SECOND)).min(1.0);
            let done = focus.progress >= 1.0;
            if done {
                focus.playing = false;
            }
            done
        };
        let texts = self.repose();
        self.show_focus_texts(texts);
        self.show_progress();
        self.request_render();
        if !done {
            self.state.focus_clock.ticking = true;
            self.on_next_frame(|now| SelectionInput::FocusTick(now).into());
        }
    }

    /// Renders hardware focus's small views again shortly, once their
    /// size has stopped changing: only the last of several asks renders.
    pub(in crate::ui) fn schedule_pictures(&mut self) {
        self.state.pictures_asked = self.state.pictures_asked.wrapping_add(1);
        let asked = self.state.pictures_asked;
        let input = self.input();
        glib::timeout_add_local_once(PICTURES_DELAY, move || {
            input.emit(SelectionInput::PicturesDue(asked).into());
        });
    }

    /// Renders hardware focus's small views offscreen, at fixed angles
    /// (closed, part way, fully open), in the view from above when there
    /// is one, each at its slot's size in device pixels. Before the slots
    /// have room they are only laid out: their first size renders them.
    pub(in crate::ui) fn render_pictures(&mut self) {
        let captions: Vec<String> = {
            let state = &self.state;
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
        self.widgets.pictures.root.set_visible(!captions.is_empty());
        self.widgets.pictures.show(&captions);
        let Some((width, height)) = self.widgets.pictures.size() else {
            return;
        };
        let backdrop = self.backdrop();
        let textures = {
            let session = &self.session;
            let (Some((_, graph)), Some(entry)) = (session.drawable(), session.scene_entry())
            else {
                return;
            };
            let state = &mut self.state;
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

    /// Offers the motion bar to open the selection's door or drawer.
    pub(in crate::ui) fn update_opening(&mut self, assembly: Option<AssemblyId>) {
        let found = {
            let session = &self.session;
            assembly.and_then(|id| {
                let (design, graph) = session.drawable()?;
                let range = graph.motion_range(id)?;
                let name = design.assembly(id)?.name.clone();
                Some((id, range, name))
            })
        };
        let amount = {
            let state = &mut self.state;
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
                    turns: range.turns(),
                })
            });
            state.opening.map_or(0.0, |o| o.amount)
        };
        match &found {
            Some((_, range, name)) => {
                let (full, value, end) = if range.turns() {
                    let degrees = range.full().to_degrees();
                    (
                        degrees,
                        amount.to_degrees(),
                        format!("{}°", degrees.round()),
                    )
                } else {
                    (range.full(), amount, format!("{} mm", range.full().round()))
                };
                self.show_motion(Some(MotionShown {
                    name: name.clone(),
                    start: tr("closed"),
                    end,
                    full: f64::from(full),
                    value: f64::from(value),
                }));
            }
            None => self.show_motion(None),
        }
        self.repose();
    }

    /// Shows the motion bar (its slider moving without calling back), or
    /// hides it.
    pub(in crate::ui) fn show_motion(&mut self, shown: Option<MotionShown>) {
        self.state.motion_shown = shown.is_some();
        self.widgets.motion.show(shown);
    }

    /// The motion bar moved: the selection opening.
    pub(in crate::ui) fn motion_moved(&mut self, value: f64) {
        {
            let state = &mut self.state;
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
    pub(in crate::ui) fn problem_moved(&mut self, value: f64) {
        let shown = {
            let state = &mut self.state;
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
    pub(in crate::ui) fn show_posed_status(&self, card: &MotionCard, amount: f32) {
        self.show_hint(&fill_in(
            &tr("Posed at contact · {name} at {amount}"),
            &[("name", &card.name), ("amount", &card.amount_text(amount))],
        ));
    }

    /// Shows the controls for a posed problem (its motion card and the
    /// shared-wood key in place of the view toolbar), or the usual ones.
    pub(in crate::ui) fn show_problem_controls(&self, problem: Option<(&MotionCard, f32, bool)>) {
        let widgets = &self.widgets;
        match problem {
            Some((card, amount, _)) => widgets.problem_card.show(Some(card), amount),
            None => widgets.problem_card.show(None, 0.0),
        }
        let posed = problem.is_some();
        let shared = problem.is_some_and(|(_, _, shared)| shared);
        let focused = self.state.focus.is_some();
        widgets.contact_key.set_visible(shared && !focused);
        widgets.offer_view_tools(!posed && !focused);
        if !posed && !focused {
            self.show_hint("");
        }
    }

    /// Poses the design where a problem happens and frames its parts.
    pub(in crate::ui) fn pose_problem(&mut self, contact: Contact, card: Option<MotionCard>) {
        let highlight = {
            let session = &self.session;
            session
                .drawable()
                .map(|(_, graph)| view::highlight(graph, &contact))
        };
        let Some(highlight) = highlight else {
            return;
        };
        let shared = !highlight.volumes.is_empty();
        let turns = card.as_ref().is_none_or(|c| c.turns());
        {
            let state = &mut self.state;
            state.opening = None;
            state.problem = Some(ShownProblem {
                contact,
                highlight,
                card: card.clone(),
                amount: None,
                turns,
            });
        }
        self.show_motion(None);
        self.repose();
        {
            let session = &self.session;
            let state = &mut self.state;
            let aspect = Self::aspect(state);
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
                self.show_problem_controls(Some((card, card.shown(), shared)));
                self.show_posed_status(card, card.shown());
            }
            None => {
                self.widgets.problem_card.show(None, 0.0);
                self.widgets.contact_key.set_visible(shared);
                self.widgets.offer_view_tools(false);
                if shared {
                    self.show_hint(&tr("Posed at contact"));
                }
            }
        }
    }

    /// Answers the finder's search with what the query finds.
    pub(in crate::ui) fn finder_search(&self, query: &str) {
        let analysis = self.widgets.files.analysis();
        let session = &self.session;
        let hits = finding::search(
            finding::Sources {
                shown: session.shown().map(|c| &**c),
                latest: session.latest().map(|c| &**c),
                analysis: analysis.as_deref(),
            },
            query,
        );
        self.widgets.finder.show_hits(hits);
    }

    /// Goes where a finder's hit says: a part is selected (and framed), a
    /// problem posed, a place in the text opened in the editor.
    pub(in crate::ui) fn finder_hit(&mut self, hit: finding::Hit, choice: Choice) {
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
                self.state.linking = true;
                self.dispatch(Command::Link(Some(selection)));
                self.state.linking = false;
                self.frame();
            }
            (Goal::Select(_) | Goal::Open(_), Choice::Preview) => {}
            (Goal::Select(selection), Choice::Go) => {
                self.select_and_frame(Some(selection));
            }
            (Goal::Select(selection), Choice::OpenText) => {
                let placed = self
                    .session
                    .shown()
                    .and_then(|shown| link::placed_at(shown, &selection));
                self.select_and_frame(Some(selection));
                if let Some(placed) = placed {
                    self.dispatch(Command::GoToSource(placed));
                }
            }
            (Goal::Open(location), Choice::Go | Choice::OpenText) => {
                self.dispatch(Command::GoToSource(location));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use relm4::{Component, ComponentController, Controller};

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// A project window on `example`, launched as the application
    /// launches one, once its first check has a result. The application
    /// is kept for as long as the window.
    fn open_window(example: &str, id: &str) -> (adw::Application, Controller<Window>) {
        let application = adw::Application::new(Some(id), gio::ApplicationFlags::NON_UNIQUE);
        // A window joins only an application that has started.
        if let Err(error) = application.register(gio::Cancellable::NONE) {
            panic!("the test's application registers: {error}");
        }
        let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(example)
            .canonicalize()
            .unwrap();
        // A layout of its own, which is never written: the people's own
        // layout is left alone.
        let options = Options {
            layout: Some(std::env::temp_dir().join("stave-selection-test-no-layout")),
            ..Options::default()
        };
        let builder = Window::builder();
        let root = builder.widget().clone();
        let widgets =
            build_widgets(&root, &application, &folder, &gio::Menu::new(), &options).unwrap();
        let controller = builder
            .launch(WindowInit {
                widgets,
                folder,
                options,
            })
            .detach();
        // The check runs on a thread of its own and answers by message.
        for _ in 0..12_000 {
            settle();
            if controller.model().session.shown().is_some() {
                return (application, controller);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("{example} was not checked in two minutes");
    }

    /// Closes the window and lets it go.
    fn close(controller: Controller<Window>) {
        controller.widget().close();
        settle();
        drop(controller);
        settle();
    }

    /// The first joint in the design that focus mode opens on.
    fn a_joint(controller: &Controller<Window>) -> Target {
        let targets: Vec<Target> = controller
            .model()
            .session
            .drawable()
            .map(|(design, _)| {
                design
                    .joints()
                    .iter()
                    .filter(|joint| !joint.pairs.is_empty())
                    .map(|joint| Target::Joint {
                        joint: joint.id,
                        pair: stave_model::solved::PairIndex::FIRST,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for target in targets {
            controller
                .sender()
                .send(SelectionInput::Focus(target).into())
                .unwrap();
            settle();
            if controller.model().state.focus.is_some() {
                return target;
            }
        }
        panic!("no joint in the design opens in focus mode");
    }

    /// What focus mode's options are, or `None` outside it.
    fn options(controller: &Controller<Window>) -> Option<(bool, bool, bool)> {
        controller.model().state.focus.as_ref().map(|focus| {
            (
                focus.options.xray,
                focus.options.tint_cut_faces,
                focus.options.fade_others,
            )
        })
    }

    #[test]
    fn focus_modes_checks_follow_people_and_not_the_windows_own_setting() {
        on_gtk_thread(checks_follow_people);
    }

    /// People's changes to focus mode's checks reach the model once the
    /// main loop runs; the window turning them on again when it opens
    /// focus mode is never heard back, and the checks end as the model
    /// has them.
    fn checks_follow_people() {
        let (_application, controller) =
            open_window("clothing_cabinet", "org.stave.SelectionTest.Checks");
        let target = a_joint(&controller);
        assert_eq!(options(&controller), Some((true, true, true)));
        let (xray, tint, fade) = {
            let panel = &controller.model().widgets.inspector.focus;
            (panel.xray.clone(), panel.tint.clone(), panel.fade.clone())
        };
        assert!(xray.is_active() && tint.is_active() && fade.is_active());

        // People turn X-ray and Fade off: queued, then heard.
        xray.set_active(false);
        fade.set_active(false);
        assert_eq!(
            options(&controller),
            Some((true, true, true)),
            "only queued until the main loop runs"
        );
        settle();
        assert_eq!(options(&controller), Some((false, true, false)));

        // Leaving and opening focus mode again turns every option on; the
        // checks show it without calling back.
        let input = controller.sender().clone();
        input.send(SelectionInput::LeaveFocus.into()).unwrap();
        input.send(SelectionInput::Focus(target).into()).unwrap();
        settle();
        assert_eq!(options(&controller), Some((true, true, true)));
        assert!(xray.is_active() && tint.is_active() && fade.is_active());

        // People turn Tint off while focus mode opens again in the same
        // turn: every message is handled in order, so the model ends as
        // the checks show.
        input.send(SelectionInput::LeaveFocus.into()).unwrap();
        input.send(SelectionInput::Focus(target).into()).unwrap();
        tint.set_active(false);
        settle();
        assert_eq!(tint.is_active(), options(&controller).is_some_and(|o| o.1));
        close(controller);
    }

    #[test]
    fn escape_leaves_focus_then_clears_the_selection() {
        on_gtk_thread(problem_then_escape);
    }

    /// Escape and the history are messages: nothing changes until the
    /// main loop runs. Escape leaves focus mode first, then clears the
    /// selection, and Back brings the selection back.
    fn problem_then_escape() {
        let (_application, controller) =
            open_window("clothing_cabinet", "org.stave.SelectionTest.Escape");
        let input = controller.sender().clone();
        let selected =
            |controller: &Controller<Window>| controller.model().session.selection().cloned();
        let target = a_joint(&controller);
        assert_eq!(controller.model().session.focus(), Some(target));
        input.send(SelectionInput::Escape.into()).unwrap();
        assert!(controller.model().session.focus().is_some(), "queued");
        settle();
        assert_eq!(
            controller.model().session.focus(),
            None,
            "Escape leaves focus"
        );
        assert!(controller.model().state.focus.is_none());

        input
            .send(SelectionInput::Select(Some(Selection::Problem(ProblemIndex(0)))).into())
            .unwrap();
        settle();
        let problem = selected(&controller);
        input.send(SelectionInput::Escape.into()).unwrap();
        settle();
        assert_eq!(selected(&controller), None, "Escape clears the selection");
        if problem.is_some() {
            input.send(SelectionInput::Back.into()).unwrap();
            settle();
            assert_eq!(selected(&controller), problem, "Back goes to it again");
        }
        close(controller);
    }
}
