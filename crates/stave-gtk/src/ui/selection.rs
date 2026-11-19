//! What follows the selection, the view and the editor: the Code panel
//! kept on the selection's line, the Details card, the 3D view's
//! description and the hints over it.

use stave_app::i18n::{fill_in, tr};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::problems::Mark;
use stave_app::session::{Command, Editor, Selection, View};
use stave_app::{inspect, link};
use stave_model::solved::AssemblyId;

use super::{Window, files, view_description, view_tools, workbench};

impl Window {
    pub(super) fn view_changed(self: &Rc<Self>) {
        let view = self.session.borrow().view();
        self.widgets.header.show_view(view);
        self.widgets.views.set_visible_child_name(view.word());
        self.sync_files();
        self.request_render();
    }

    /// `true` when the 3D view is on screen: the design.
    pub(super) fn shows_3d(&self) -> bool {
        match self.session.borrow().view() {
            View::Design => true,
            View::CutList => false,
        }
    }

    /// The editor opened or closed. Opening shows the Code panel wherever
    /// it is docked, with the selection's line; closing gives the keyboard
    /// back to the 3D view (the panel stays where it is).
    pub(super) fn editor_changed(self: &Rc<Self>) {
        let open = self.session.borrow().editor() == Editor::Open;
        if open {
            self.widgets.workbench.present(workbench::PanelId::Code);
            self.sync_files();
            self.widgets.files.focus_text();
        } else {
            gtk::prelude::GtkWindowExt::set_focus(
                &self.widgets.window,
                Some(self.widgets.viewport.upcast_ref::<gtk::Widget>()),
            );
        }
        self.request_render();
    }

    /// The editor follows the selection while it is open: a place in a
    /// file is shown; a part, assembly or piece of hardware selected in
    /// the 3D view, the model tree or the finder moves the text to the
    /// line that places it.
    pub(super) fn sync_files(self: &Rc<Self>) {
        let (open, selection, placed) = {
            let session = self.session.borrow();
            let selection = session.selection().cloned();
            let placed = session
                .shown()
                .zip(selection.as_ref())
                .and_then(|(shown, selection)| link::placed_at(shown, selection));
            (session.editor() == Editor::Open, selection, placed)
        };
        let files = &self.widgets.files;
        match (&selection, placed) {
            (Some(Selection::Location(location)), _) if open => files.show_location(location),
            // A part, assembly or piece of hardware chosen anywhere opens
            // the file that places it, at that line, in the Code panel
            // (forward unless it would cover the 3D view; the keyboard
            // stays where it is). A closed Code panel stays closed: a
            // first launch doesn't show code until it is asked for.
            (Some(_), Some(placed)) if !self.state.borrow().linking => {
                files.reveal(&placed);
                let workbench = &self.widgets.workbench;
                if workbench.is_open(workbench::PanelId::Code) {
                    workbench.bring_forward_beside_view(workbench::PanelId::Code);
                }
            }
            _ => {}
        }
    }

    /// The editor's cursor rests on a name: select what it means in the
    /// design, without a history entry. A name the 3D view can't show
    /// leaves the selection as it is.
    pub(super) fn text_linked(self: &Rc<Self>, found: files::CursorName) {
        let (analysis, symbol) = match found {
            files::CursorName::Name(analysis, symbol) => (analysis, symbol),
            // A broken name: the 3D view lets go, and the chip goes.
            files::CursorName::Unresolved => {
                let linked =
                    self.session.borrow().selection().is_some_and(|s| {
                        !matches!(s, Selection::Location(_) | Selection::Problem(_))
                    });
                if linked {
                    self.state.borrow_mut().linking = true;
                    self.dispatch(Command::Link(None));
                    self.state.borrow_mut().linking = false;
                }
                return;
            }
            files::CursorName::Other => return,
        };
        let selection = {
            let session = self.session.borrow();
            session.drawable().and_then(|(design, _)| {
                link::selection_for(design, &analysis, symbol, session.selection())
            })
        };
        if let Some(selection) = selection {
            self.state.borrow_mut().linking = true;
            self.dispatch(Command::Link(Some(selection)));
            self.state.borrow_mut().linking = false;
        }
    }

    pub(super) fn selection_changed(self: &Rc<Self>) {
        let (selection, inspection, problem, focused) = {
            let session = self.session.borrow();
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
        let widgets = &self.widgets;
        let linked = {
            let session = self.session.borrow();
            session
                .drawable()
                .zip(selection.as_ref())
                .and_then(|((design, _), selection)| {
                    let placed = link::placed_at(session.shown()?, selection)?;
                    link::chip(design, selection, &placed)
                })
        };
        widgets
            .workbench
            .status
            .set_label(linked.as_deref().unwrap_or(""));
        widgets.tree.show_selection(selection.as_ref());
        widgets.problems.show_selected(match &selection {
            Some(Selection::Problem(index)) => Some(index.0),
            _ => None,
        });
        self.sync_files();
        if focused {
            self.show_card();
            return;
        }
        let explode = self.session.borrow().explode().get();
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
        widgets.inspector.show(inspection.as_ref());
        // Pose a problem where it happens, or offer to open the selection.
        match problem.and_then(|(problem, details)| Some((problem.contact?, details.motion))) {
            Some((contact, card)) => self.pose_problem(contact, card),
            None => {
                self.state.borrow_mut().problem = None;
                self.show_problem_controls(None);
                let moves_with = match &selection {
                    Some(Selection::Assembly(id)) => Some(*id),
                    _ => inspection.and_then(|i| i.moves_with),
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
    pub(super) fn describe_view(&self) {
        let (name, description) = {
            let session = self.session.borrow();
            let selected = session.selection().and_then(|selection| match selection {
                Selection::Problem(_) => session.selected_problem().map(|problem| {
                    fill_in(
                        &tr("The problem “{message}”"),
                        &[("message", &problem.message)],
                    )
                }),
                _ => session
                    .shown()
                    .and_then(|shown| inspect::inspect(shown, selection))
                    .map(|inspection| inspection.title),
            });
            (
                fill_in(
                    &tr("3D view of {project}"),
                    &[("project", &session.title())],
                ),
                view_description(selected.as_deref(), &session.check_status().pill().label),
            )
        };
        self.widgets.viewport.describe(&name, &description);
    }

    /// The Details panel always shows the selection's account; focus mode
    /// brings it forward, since its controls are there.
    pub(super) fn show_card(self: &Rc<Self>) {
        if self.session.borrow().focus().is_some() {
            self.widgets.workbench.present(workbench::PanelId::Details);
        }
        self.update_face_button();
    }

    /// A short line at the top of the 3D view ("Focused on one joint · Esc
    /// to go back"); empty hides it.
    pub(super) fn show_hint(&self, text: &str) {
        self.widgets.hint.set_label(text);
        self.widgets.hint_box.set_visible(!text.is_empty());
    }

    /// The dock's chips: every assembly that moves, with its mark, the
    /// one the motion slider shows marked current.
    pub(super) fn show_moves(self: &Rc<Self>) {
        let (moving, current, sliding) = {
            let session = self.session.borrow();
            let state = self.state.borrow();
            let moving: Vec<(AssemblyId, String, Option<Mark>)> = session
                .drawable()
                .map(|(design, graph)| {
                    design
                        .assemblies
                        .iter()
                        .filter(|a| a.parent.is_none() && graph.motion_range(a.id).is_some())
                        .map(|a| (a.id, a.name.clone(), state.marks.assembly(a.id)))
                        .collect()
                })
                .unwrap_or_default();
            (
                moving,
                state.opening.map(|o| o.assembly),
                self.widgets.motion.root.is_visible(),
            )
        };
        // While the selection's own slider shows, the chips step aside
        // (V2C_Spatial.dc.html): there isn't room for both.
        self.widgets
            .dock
            .show_moves(if sliding { &[] } else { &moving }, current);
    }
}
