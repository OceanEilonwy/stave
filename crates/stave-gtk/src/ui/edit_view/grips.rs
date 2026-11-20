//! Sizes in the Edit panel (`docs/gui.md`, "Sizes"; `docs/editing.md`,
//! "Grips"): the sizes of a selected assembly, of the envelope or of a
//! split's boundaries, one row each in the Edit panel's Sizes card
//! (`sizes.rs`), each a number field dragged sideways to change it or
//! clicked to type a value; a drag moves the value live with its ghost
//! and verdict; the sizes to pick from when a size follows several, and
//! the switch that lets the solver move a param. Nothing is drawn on the
//! model: what the drag changes, the sizes to pick from, the stop and the
//! verdict are in the change card's Size section (`grip_panel.rs`),
//! whose Apply writes it.
//!
//! ```text
//! selection / a click in a space ──ListGrips──▶ GripsChanged ──▶ the Sizes card's rows
//!                                               StopsJob on a worker ──▶ GripStopsFound ──▶ each field's range
//! drag a field ──BeginDrag(i)──▶ move ──DragTo(value)──▶ DragMoved / VerdictChanged ──▶ the field, the Size section, the ghost
//! let go ──▶ Add when it fits          Escape or Cancel ──▶ nothing written
//! click a field ──▶ entry ──Enter──▶ TypeGripValue ──▶ Add when it fits, else why, still editable
//! the switch (⋮) ──▶ MakeSolved / MakePlain ──▶ Edit panel ──Apply──▶ Add
//! ```
//!
//! The session decides everything: what a grip moves, where it snaps, what
//! stops it, the verdict and the text written; how far a pixel of a drag
//! moves a value is `stave_app::grip_values::scrubbed`. Here a field's
//! travel becomes a value to send, and what the session holds is shown.

use stave_app::i18n::{fill_in, tr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use stave_app::change::{Change, ChangeRequest};
use stave_app::edit::{Picked, Verdict};
use stave_app::editing::{EditCommand, EditEvent, Progress, parse_value};
use stave_app::gripping::{FoundStops, GripList, ListedGrip};
use stave_app::grips::{FollowKind, GripAction, GripSelection};
use stave_app::session::{Command, Event, Selection};
use stave_lang::ast::Ident;
use stave_model::ids::{AssemblyId, ParamId, PartId};
use stave_model::trace::Amount;
use stave_scene::Item;

use super::grip_panel::{
    GripOptions, GripPanelOutput, GripSection, PanelAction, SwitchAsk, SwitchMenu,
};
use super::grip_text::{
    GripLook, Held, amount_like, amount_words, grip_look, grip_value, held_words, panel_model,
    solver_switches, typed_check, typed_refusal,
};
use super::sizes::SizesCard;
use crate::ui::update::editing::EditingInput;
use crate::ui::{Window, WindowInput};

/// The sizes' card and section, and what only they need.
pub(in crate::ui) struct GripView {
    /// The size a drag moves, as a section of the change's card; its
    /// actions come to the window as [`EditingInput::GripPanel`].
    panel: GripSection,
    /// The Sizes card of the Edit panel: a row per size.
    pub(super) sizes: SizesCard,
    state: GripState,
}

/// What the grips keep between events.
#[derive(Default)]
struct GripState {
    /// What the sizes shown are of.
    shown_for: Option<GripSelection>,
    /// Stops the running stops search.
    cancel: Option<Arc<AtomicBool>>,
    /// The pointer holds a field.
    gesture: Option<Gesture>,
    /// The value to send next, while one is being tried.
    pending: Option<Amount>,
    /// A `DragTo` is waiting to be sent.
    sending: bool,
    /// The last value sent.
    last_sent: Option<Amount>,
    /// The grip whose change (a drag, or a param made solved or plain) the
    /// grips started and hold open.
    open: Option<usize>,
    /// The drag was held at an end of its range.
    held: Option<Held>,
    /// The grip whose value is being typed.
    typing: Option<usize>,
    /// A listing from the new design is on its way.
    relist_queued: bool,
}

/// The pointer holding a field.
#[derive(Debug, Clone, Copy)]
struct Gesture {
    /// The grip.
    index: usize,
    /// Its value when it was pressed.
    from: Amount,
}

/// What a grip's options menu asked, as the window's message.
pub(super) fn switch_asked(index: usize, ask: SwitchAsk, menu: SwitchMenu) -> EditingInput {
    match ask {
        SwitchAsk::MakePlain(param) => EditingInput::GripMakePlain { index, param },
        SwitchAsk::MakeSolved { param, low, high } => EditingInput::GripMakeSolved {
            index,
            param,
            low,
            high,
            menu,
        },
    }
}

impl GripView {
    /// The card and the section, hidden until sizes are listed; their
    /// actions go to `input`.
    pub(in crate::ui) fn new(input: &relm4::Sender<WindowInput>) -> GripView {
        GripView {
            panel: GripSection::new(input, |output| match output {
                GripPanelOutput::Action(action) => EditingInput::GripPanel(action).into(),
            }),
            sizes: SizesCard::new(input),
            state: GripState::default(),
        }
    }
}

/// The top-level assembly an assembly is part of (itself when it is
/// one).
fn top_level(design: &stave_model::solved::SolvedDesign, id: AssemblyId) -> Option<AssemblyId> {
    let mut found = id;
    for _ in 0..=design.assemblies().len() {
        match design.assembly(found)?.parent {
            Some(parent) => found = parent,
            None => return Some(found),
        }
    }
    None
}

impl Window {
    /// The grips' panel asked for something.
    pub(in crate::ui) fn grip_panel_asked(&mut self, action: PanelAction) {
        match action {
            PanelAction::Pick(param) => self.pick_later(param),
            PanelAction::Switch {
                index,
                ask: SwitchAsk::MakePlain(param),
            } => self.grip_make_plain(index, param),
            PanelAction::Switch {
                index,
                ask: SwitchAsk::MakeSolved { param, low, high },
            } => self.grip_make_solved(index, param, (&low, &high), &SwitchMenu::Panel),
            PanelAction::Type(text) => {
                let open = self.session.change().and_then(drag_index).or(self
                    .editing
                    .grips
                    .state
                    .open);
                if let Some(index) = open {
                    self.apply_typed(index, &text);
                }
            }
        }
    }

    // ---- what the grips are of ----

    /// The selection changed: an assembly (or a part of one) shows the
    /// grips of its top-level assembly; anything else hides an assembly's
    /// grips (a space's stay until another click).
    pub(in crate::ui) fn grips_follow_selection(&mut self) {
        let wanted = {
            let session = &self.session;
            let assembly = match (session.selection(), session.drawable()) {
                (Some(Selection::Assembly(id)), Some((design, _))) => top_level(design, *id),
                (Some(Selection::Part(part)), Some((design, graph))) => graph
                    .node_of(Item::Part(*part))
                    .and_then(|node| graph.node(node))
                    .and_then(|node| node.assemblies.first().copied())
                    .and_then(|id| top_level(design, id)),
                _ => None,
            };
            match (assembly, self.editing.grips.state.shown_for) {
                (Some(id), _) => Some(GripSelection::Assembly(id)),
                (None, Some(region @ (GripSelection::Envelope | GripSelection::Split(_)))) => {
                    session.selection().is_none().then_some(region)
                }
                (None, Some(GripSelection::Assembly(_)) | None) => None,
            }
        };
        self.show_grips_soon(wanted);
    }

    /// Lists the grips of `wanted` once the window has drawn what is
    /// already asked for (a listing prepares the project again).
    fn show_grips_soon(&mut self, wanted: Option<GripSelection>) {
        if self.editing.grips.state.shown_for == wanted && self.grips_listed(wanted) {
            return;
        }
        if wanted.is_none() {
            self.show_grips(None);
            return;
        }
        let input = self.input();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            input.emit(EditingInput::ShowGrips(wanted).into());
        });
    }

    /// `true` when the session's listing is of `wanted`.
    fn grips_listed(&self, wanted: Option<GripSelection>) -> bool {
        let session = &self.session;
        match (session.grips(), wanted) {
            (Some(grips), Some(wanted)) => grips.selection() == wanted,
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        }
    }

    /// Lists the grips of `wanted` now (none: hides them). A selection
    /// with no grips (a space that is not a piece of a split) shows none,
    /// quietly.
    pub(in crate::ui) fn show_grips(&mut self, wanted: Option<GripSelection>) {
        self.stop_stops_search();
        self.editing.grips.state.shown_for = wanted;
        let Some(selection) = wanted else {
            self.clear_grip_marks();
            self.show_card();
            self.request_render();
            return;
        };
        {
            let session = &mut self.session;
            session.set_edit_files(Arc::new(self.widgets.files.edit_files()));
        }
        let events = self
            .session
            .apply(Command::Edit(EditCommand::ListGrips(selection)));
        // A space with no boundaries, or a design that doesn't place, has
        // no grips: nothing to say.
        let events: Vec<Event> = events
            .into_iter()
            .filter(|event| !matches!(event, Event::Edit(EditEvent::Rejected(_))))
            .collect();
        if self
            .session
            .grips()
            .is_none_or(|grips| grips.selection() != selection)
        {
            self.editing.grips.state.shown_for = None;
            self.clear_grip_marks();
        }
        self.handle(events);
    }

    /// An edit started: a click in the envelope or in a piece of a split
    /// shows its grips beside the menu.
    fn grips_for_edit_start(&mut self) {
        let region = {
            let session = &self.session;
            let region = match session.edit().map(|edit| edit.picked()) {
                Some(Picked::Region(region)) => Some(*region),
                Some(Picked::Face(_) | Picked::Between(_) | Picked::Group(_)) | None => None,
            };
            region
                .zip(session.drawable())
                .and_then(|(region, (_, graph))| {
                    let name = &graph.region(region)?.name;
                    if name == stave_app::naming::ENVELOPE_REGION {
                        Some(GripSelection::Envelope)
                    } else if name.contains('[') {
                        Some(GripSelection::Split(region))
                    } else {
                        None
                    }
                })
        };
        let keep = matches!(
            self.editing.grips.state.shown_for,
            Some(GripSelection::Assembly(_))
        ) && region.is_none()
            && self.session.selection().is_some();
        if !keep {
            self.show_grips_soon(region);
        }
    }

    // ---- events ----

    /// Redraws what an edit event changes for the grips. Called before the
    /// edit view's own handling of it.
    pub(in crate::ui) fn grip_event(&mut self, event: &EditEvent) {
        match event {
            EditEvent::GripsChanged => {
                let (listed, shown_for) = {
                    let session = &self.session;
                    (
                        session.grips().map(GripList::selection),
                        self.editing.grips.state.shown_for,
                    )
                };
                match (listed, shown_for) {
                    (Some(listed), Some(shown)) if listed == shown => {
                        self.build_grip_marks();
                        self.start_stops_search();
                    }
                    // A commit drops the listing: list again from the new
                    // design. The rows stay until then (they do nothing
                    // without a listing), so the panel doesn't flicker.
                    (None, Some(_)) => self.relist_grips_soon(),
                    (Some(_), _) | (None, None) => {}
                }
            }
            EditEvent::GripStopsFound(_) => {
                let busy = {
                    let state = &self.editing.grips.state;
                    state.gesture.is_some() || state.typing.is_some() || state.open.is_some()
                };
                if !busy {
                    self.build_grip_marks();
                }
            }
            EditEvent::Started => self.grips_for_edit_start(),
            EditEvent::Committed => {
                let state = &mut self.editing.grips.state;
                state.open = None;
                state.typing = None;
                state.gesture = None;
                state.held = None;
                state.pending = None;
                state.last_sent = None;
            }
            EditEvent::Cancelled => {
                let still_open = self.session.change().is_some_and(|change| {
                    matches!(
                        change.request(),
                        ChangeRequest::Grip { .. } | ChangeRequest::ParamKind { .. }
                    )
                });
                let (was_open, region) = {
                    let grips = &mut self.editing.grips;
                    let state = &mut grips.state;
                    let was_open = state.open.is_some();
                    if !still_open {
                        state.open = None;
                        state.gesture = None;
                        state.held = None;
                        state.pending = None;
                        state.last_sent = None;
                        state.typing = None;
                    }
                    grips.sizes.close_entries();
                    (
                        was_open,
                        matches!(
                            state.shown_for,
                            Some(GripSelection::Envelope | GripSelection::Split(_))
                        ),
                    )
                };
                // The menu of a space was let go (not a drag on its
                // grips): its grips go with it.
                if !was_open && !still_open && region && self.session.edit().is_none() {
                    self.show_grips(None);
                }
            }
            EditEvent::ChangeStarted
            | EditEvent::DragMoved
            | EditEvent::ParamPicked
            | EditEvent::VerdictChanged
            | EditEvent::MenuChanged
            | EditEvent::Chosen
            | EditEvent::PlaceBound
            | EditEvent::SettingChanged
            | EditEvent::Renamed
            | EditEvent::RepeatChanged
            | EditEvent::DependantsIncluded
            | EditEvent::FixApplied
            | EditEvent::OptionsChanged
            | EditEvent::Rejected(_) => {}
        }
        self.refresh_sizes();
        self.request_render();
    }

    /// A new result (a commit, or the text changed in the editor): the
    /// grips shown are listed again from it, unless one is being dragged
    /// or typed (its change checks the text it began from when written).
    pub(in crate::ui) fn grips_result_changed(&mut self) {
        let busy = {
            let state = &self.editing.grips.state;
            state.shown_for.is_none()
                || state.gesture.is_some()
                || state.typing.is_some()
                || state.open.is_some()
        };
        if !busy {
            self.relist_grips_soon();
        }
    }

    /// Lists the grips shown again once the events in hand are handled.
    pub(in crate::ui) fn relist_grips_soon(&mut self) {
        if std::mem::replace(&mut self.editing.grips.state.relist_queued, true) {
            return;
        }
        let input = self.input();
        glib::timeout_add_local_once(Duration::from_millis(30), move || {
            input.emit(EditingInput::RelistGrips.into());
        });
    }

    /// Lists the grips shown again, as asked by [`Window::relist_grips_soon`].
    pub(in crate::ui) fn relist_grips(&mut self) {
        let shown = {
            let state = &mut self.editing.grips.state;
            state.relist_queued = false;
            state.shown_for
        };
        if shown.is_some() {
            self.show_grips(shown);
        }
    }

    // ---- the stops, off the main thread ----

    fn stop_stops_search(&mut self) {
        if let Some(cancel) = self.editing.grips.state.cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Searches the listed grips' stops on a worker thread, handing each
    /// answer to the session as it comes.
    fn start_stops_search(&mut self) {
        self.stop_stops_search();
        let Some(job) = self.session.grip_stops_job() else {
            return;
        };
        if job.searches().is_empty() {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.editing.grips.state.cancel = Some(cancel.clone());
        let (sender, receiver) = async_channel::unbounded::<FoundStops>();
        gio::spawn_blocking(move || {
            let _ = job.run(&cancel, &mut |found| {
                let _ = sender.send_blocking(found);
            });
        });
        let input = self.input();
        glib::spawn_future_local(async move {
            while let Ok(found) = receiver.recv().await {
                if input
                    .send(EditingInput::GripStopsFound(Box::new(found)).into())
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    // ---- the widgets on the model ----

    // ---- the rows ----

    fn clear_grip_marks(&mut self) {
        self.editing.grips.sizes.clear();
        self.editing.grips.state.typing = None;
        self.update_edit_panel();
    }

    /// Makes a row for every listed grip, in the Sizes card.
    fn build_grip_marks(&mut self) {
        let (title, listed, part) = {
            let session = &self.session;
            let Some(grips) = session.grips() else {
                return;
            };
            let title = match grips.selection() {
                GripSelection::Assembly(id) => session
                    .drawable()
                    .and_then(|(design, _)| design.assembly(id))
                    .map(|assembly| assembly.name.clone())
                    .unwrap_or_default(),
                GripSelection::Envelope => tr("the space"),
                GripSelection::Split(region) => session
                    .drawable()
                    .and_then(|(_, graph)| graph.region(region))
                    .map(|region| region.name.clone())
                    .unwrap_or_default(),
            };
            let listed: Vec<(usize, ListedGrip)> =
                grips.grips().iter().cloned().enumerate().collect();
            let part: Option<PartId> = match session.selection() {
                Some(Selection::Part(part)) => Some(*part),
                _ => None,
            };
            (title, listed, part)
        };
        self.editing.grips.sizes.show(&title, &listed, part);
        self.editing.grips.state.typing = None;
        self.update_edit_panel();
        self.refresh_sizes();
    }

    /// Shows each field's value (the dragged one's where it was moved),
    /// and the change card's Size section.
    pub(in crate::ui) fn refresh_sizes(&mut self) {
        {
            let session = &self.session;
            let grips = &self.editing.grips;
            let state = &grips.state;
            let open = session.change().and_then(drag_index).or(state.open);
            if let Some(list) = session.grips() {
                for (index, listed) in list.grips().iter().enumerate() {
                    if state.typing == Some(index) {
                        continue;
                    }
                    let moved = (open == Some(index))
                        .then(|| {
                            session
                                .change()
                                .and_then(|change| drag_target(change, index))
                                .or(state.pending)
                        })
                        .flatten();
                    let value = moved.unwrap_or_else(|| grip_value(listed));
                    grips.sizes.set_value(index, &amount_words(value));
                }
            }
            grips
                .sizes
                .set_scrubbing(state.gesture.map(|gesture| gesture.index));
        }
        self.show_grip_section();
    }

    // ---- dragging ----

    /// The listed grip at `index`.
    fn listed_grip(&self, index: usize) -> Option<ListedGrip> {
        self.session.grips()?.grips().get(index).cloned()
    }

    /// The value a drag on `index` is at now: where it was moved, or its
    /// value.
    fn grip_target(&self, index: usize) -> Option<Amount> {
        let session = &self.session;
        let moved = session
            .change()
            .and_then(|change| drag_target(change, index));
        moved.or_else(|| Some(grip_value(session.grips()?.grips().get(index)?)))
    }

    /// Starts (or goes on with) a drag on `index`: a drag already open on
    /// the same grip goes on from where it is.
    fn begin_grip_drag(&mut self, index: usize) {
        let going_on = self
            .session
            .change()
            .is_some_and(|change| drag_index(change) == Some(index));
        if going_on {
            return;
        }
        {
            let state = &mut self.editing.grips.state;
            state.open = Some(index);
            state.held = None;
            state.last_sent = None;
            state.pending = None;
        }
        self.dispatch(Command::Edit(EditCommand::BeginDrag(index)));
        // A size that follows several plain params lists them at once,
        // each with the value it would take, to pick from first.
        let choices = self
            .listed_grip(index)
            .filter(|listed| match &listed.grip.action {
                GripAction::Follows(follows) => {
                    let plain = follows.follows.iter().filter(|value| {
                        value.kind == FollowKind::Plain
                            && value.coefficient.is_some()
                            && value.path.is_some()
                    });
                    let solved = follows
                        .follows
                        .iter()
                        .any(|value| value.kind == FollowKind::Solved);
                    plain.count() > 1 && !solved
                }
                GripAction::Moves(_) => false,
            });
        if let Some(listed) = choices {
            let value = grip_value(&listed);
            self.editing.grips.state.last_sent = Some(value);
            self.dispatch(Command::Edit(EditCommand::DragTo(value)));
        }
    }

    /// The pointer began to drag a field.
    pub(in crate::ui) fn grip_pressed(&mut self, index: usize) {
        self.begin_grip_drag(index);
        let Some(from) = self.grip_target(index) else {
            return;
        };
        self.editing.grips.state.gesture = Some(Gesture { index, from });
        self.refresh_sizes();
        self.request_render();
    }

    /// The pointer moved `dx` pixels sideways since it began to drag.
    pub(in crate::ui) fn grip_moved(&mut self, dx: f64, fine: bool, round: bool) {
        let Some(gesture) = self.editing.grips.state.gesture else {
            return;
        };
        let pixels = crate::convert::int_of(dx);
        let Some(wanted) = stave_app::grip_values::scrubbed(gesture.from, pixels, fine) else {
            return;
        };
        self.grip_wanted(gesture.index, wanted, round);
    }

    /// Arrow keys on a field: a step of its resolution each way.
    pub(in crate::ui) fn grip_stepped(&mut self, index: usize, steps: i32, round: bool) {
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        self.begin_grip_drag(index);
        let Some(now) = self.grip_target(index) else {
            return;
        };
        let Some(wanted) = stave_app::grip_values::stepped(&listed, now, steps, round) else {
            return;
        };
        self.grip_wanted(index, wanted, round);
        self.flush_drag();
    }

    /// Moves a drag toward `wanted`: snapped to the grip's steps (or to
    /// round numbers), held inside its range, and sent once the last try
    /// is done.
    fn grip_wanted(&mut self, index: usize, wanted: Amount, round: bool) {
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        let (target, held) = stave_app::grip_values::drag_target(&listed, wanted, round);
        let send = {
            let state = &mut self.editing.grips.state;
            state.held = held;
            if state.last_sent == Some(target) && state.pending.is_none() {
                false
            } else {
                state.pending = Some(target);
                !std::mem::replace(&mut state.sending, true)
            }
        };
        if send {
            // Once the events in hand are handled.
            self.send(EditingInput::FlushDrag);
        }
        self.refresh_sizes();
        self.request_render();
    }

    /// Sends the value waiting, if any.
    pub(in crate::ui) fn flush_drag(&mut self) {
        let pending = {
            let state = &mut self.editing.grips.state;
            state.sending = false;
            let pending = state.pending.take();
            if pending.is_some() {
                state.last_sent = pending;
            }
            pending
        };
        if let Some(target) = pending {
            self.dispatch(Command::Edit(EditCommand::DragTo(target)));
        }
    }

    /// The pointer let the field go: writes the drag when it fits;
    /// otherwise the Size section says why and offers Cancel.
    pub(in crate::ui) fn grip_released(&mut self) {
        self.flush_drag();
        self.editing.grips.state.gesture = None;
        let fits = self
            .session
            .change()
            .is_some_and(|change| drag_index(change).is_some() && fits(change.progress()));
        if fits {
            self.send_edit_later(EditCommand::Add);
        }
        self.refresh_sizes();
        self.request_render();
    }

    /// A pick from the sizes a follows grip could change, sent once the
    /// panel's own signal is done.
    fn pick_later(&mut self, param: ParamId) {
        self.send(EditingInput::Dispatch(Command::Edit(
            EditCommand::PickParam(param),
        )));
    }

    /// A grip's range switch turned off: the size is written plain again.
    pub(in crate::ui) fn grip_make_plain(&mut self, index: usize, param: Ident) {
        self.editing.grips.state.open = Some(index);
        self.send_edit_later(EditCommand::MakePlain(param));
    }

    /// "Make it solved" on a grip's menu, with the range typed.
    pub(in crate::ui) fn grip_make_solved(
        &mut self,
        index: usize,
        param: Ident,
        (low, high): (&str, &str),
        menu: &SwitchMenu,
    ) {
        match (parse_value(low), parse_value(high)) {
            (Ok(low), Ok(high)) => {
                match menu {
                    SwitchMenu::Row(menu) => menu.popdown(),
                    SwitchMenu::Panel => self.editing.grips.panel.close_options(),
                }
                self.editing.grips.state.open = Some(index);
                self.send_edit_later(EditCommand::MakeSolved { param, low, high });
            }
            (Err(error), _) | (_, Err(error)) => {
                self.edit_toast(&fill_in(
                    &tr("That range can't be read: {reason}"),
                    &[("reason", &error.0)],
                ));
            }
        }
    }

    // ---- typing ----

    // ---- typing ----

    /// The field turns into an entry holding the value.
    pub(in crate::ui) fn open_entry(&mut self, index: usize) {
        self.editing.grips.state.typing = Some(index);
        self.editing.grips.sizes.open_entry(index);
    }

    /// Escape in the entry: back to the number; a value typed and tried is
    /// dropped.
    pub(in crate::ui) fn close_typing(&mut self, index: usize) {
        let typed = {
            let grips = &mut self.editing.grips;
            grips.sizes.close_entry(index);
            grips.state.typing = None;
            grips.state.open == Some(index)
        };
        if typed {
            self.send_edit_later(EditCommand::Cancel);
        }
        self.refresh_sizes();
    }

    /// Enter in the entry: the value is tried through the same check as a
    /// drag; it is written when it fits, else the entry says why and stays.
    pub(in crate::ui) fn apply_typed(&mut self, index: usize, text: &str) {
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        // A value the session would write differently (past an end of the
        // range, off the resolution) is refused before it is tried: what
        // is written is always the number typed.
        let typed = amount_like(grip_value(&listed), text.trim())
            .and_then(|value| typed_check(&listed, value).map(|()| value));
        let refusal = match typed {
            Err(words) => {
                // A value tried before and still shown in the panel goes,
                // so the panel never shows another number than the entry.
                self.drop_typed_change(index);
                Some(words)
            }
            Ok(value) => {
                {
                    let state = &mut self.editing.grips.state;
                    state.open = Some(index);
                    state.typing = Some(index);
                    state.held = None;
                }
                self.dispatch(Command::Edit(EditCommand::TypeGripValue {
                    grip: index,
                    value,
                }));
                let (progress, written) = {
                    let session = &self.session;
                    let change = session.change();
                    (
                        change.map(|change| change.progress().clone()),
                        change.and_then(|change| typed_written(&listed, change, index)),
                    )
                };
                match (progress, written) {
                    // Whatever the verdict, the session would write
                    // another number: refused, and its try dropped.
                    (_, Some(written)) if written.raw() != value.raw() => {
                        self.drop_typed_change(index);
                        Some(fill_in(
                            &tr("Can't be {value}: it would be written as {written}"),
                            &[
                                ("value", &amount_words(value)),
                                ("written", &amount_words(written)),
                            ],
                        ))
                    }
                    (Some(progress), _) => typed_refusal(&progress),
                    (None, _) => Some(tr("it can't be tried")),
                }
            }
        };
        self.show_typed_result(index, text, refusal);
    }

    /// A typed value's try that is still open on grip `index` is
    /// cancelled; the entry stays open.
    fn drop_typed_change(&mut self, index: usize) {
        let open = self.session.change().and_then(drag_index) == Some(index);
        if open {
            self.dispatch(Command::Edit(EditCommand::Cancel));
        }
    }

    /// Shows what became of a typed value: written (Add), or why not,
    /// under the entry, which stays open holding the text typed.
    fn show_typed_result(&mut self, index: usize, text: &str, refusal: Option<String>) {
        match refusal {
            Some(words) => {
                self.editing.grips.state.typing = Some(index);
                self.editing.grips.sizes.show_refusal(index, text, &words);
            }
            None => {
                self.editing.grips.state.typing = None;
                self.editing.grips.sizes.close_entry(index);
                self.dispatch(Command::Edit(EditCommand::Add));
            }
        }
        self.refresh_sizes();
    }

    /// Draws the size section of the change's card for a change the grips
    /// started (a drag, or a param made solved or plain), with the grip's
    /// options.
    fn show_grip_section(&mut self) {
        let session = &self.session;
        let grips = &self.editing.grips;
        let (stop, open) = {
            let state = &grips.state;
            let open = session.change().and_then(drag_index).or(state.open);
            let stop = state
                .held
                .zip(open)
                .and_then(|(held, index)| held_words(session.grips()?.grips().get(index)?, held));
            (stop, open)
        };
        let model = session.change().and_then(|change| {
            panel_model(
                change,
                |index| session.grips()?.grips().get(index).cloned(),
                false,
                stop,
            )
        });
        let Some(model) = model else {
            grips.panel.root.set_visible(false);
            return;
        };
        let slot = &self.editing.changes.card.grip_slot;
        if grips.panel.root.parent().as_ref() != Some(slot.upcast_ref()) {
            if let Some(parent) = grips.panel.root.parent().and_downcast::<gtk::Box>() {
                parent.remove(&grips.panel.root);
            }
            slot.append(&grips.panel.root);
        }
        // The grip's options (⋮), which the section makes again only for
        // another grip.
        let options = open.and_then(|index| {
            let listed = session.grips()?.grips().get(index)?;
            let switches = solver_switches(listed);
            (grip_look(listed) != GripLook::Locked && !switches.is_empty())
                .then_some(GripOptions { index, switches })
        });
        grips.panel.show(&model, options);
        grips.panel.root.set_visible(true);
    }

    // ---- driving it without a pointer (scripts) ----

    /// A drag on grip `index` to `value`, as the pointer would make it,
    /// still held.
    pub(in crate::ui) fn drag_grip_to(&mut self, index: usize, value: Amount) {
        self.grip_pressed(index);
        self.grip_wanted(index, value, false);
        self.flush_drag();
    }

    /// Lets the held field go.
    pub(in crate::ui) fn release_grip(&mut self) {
        self.flush_drag();
        self.editing.grips.state.gesture = None;
        let fits = self
            .session
            .change()
            .is_some_and(|change| drag_index(change).is_some() && fits(change.progress()));
        if fits {
            self.dispatch(Command::Edit(EditCommand::Add));
        }
        self.refresh_sizes();
    }

    /// Types a value into grip `index`'s field and presses Enter.
    pub(in crate::ui) fn type_grip(&mut self, index: usize, text: &str) {
        self.open_entry(index);
        self.editing.grips.sizes.type_text(index, text);
        self.apply_typed(index, text);
    }

    /// Opens grip `index`'s menu with the solver's switch.
    pub(in crate::ui) fn open_grip_menu(&self, index: usize) -> bool {
        self.editing.grips.sizes.open_menu(index)
    }

    /// Marks a change of a param's kind as started from grip `index`.
    pub(in crate::ui) fn grip_change_opened(&mut self, index: usize) {
        self.editing.grips.state.open = Some(index);
    }

    /// The listed grip numbered `index`, for a script.
    pub(in crate::ui) fn grip_at(&self, index: usize) -> Option<ListedGrip> {
        self.listed_grip(index)
    }
}

fn drag_index(change: &Change) -> Option<usize> {
    match change.request() {
        ChangeRequest::Grip { index } => Some(*index),
        _ => None,
    }
}

/// Where a drag on `index` was last moved to.
fn drag_target(change: &Change, index: usize) -> Option<Amount> {
    if drag_index(change) != Some(index) {
        return None;
    }
    match change.detail() {
        stave_app::change::ChangeDetail::Grip(detail) => {
            // A movable grip's own param, as written (snapped); a size
            // that follows, the size the written values reach.
            let written = detail
                .changes
                .iter()
                .find(|change| change.path == detail.label)
                .map(|change| change.to);
            match change.progress() {
                Progress::Tried(_) => written.or(detail.reached).or(detail.target),
                Progress::NeedsChoice(_) | Progress::Unwritable(_) => detail.target,
            }
        }
        _ => None,
    }
}

/// The value a typed movable grip's change would write, when it is open
/// on `index`; `None` for a size that follows params (its params are
/// written, and the size they reach is shown in the panel).
fn typed_written(listed: &ListedGrip, change: &Change, index: usize) -> Option<Amount> {
    match listed.grip.action {
        GripAction::Moves(_) => drag_target(change, index),
        GripAction::Follows(_) => None,
    }
}

/// `true` when a change fits.
fn fits(progress: &Progress) -> bool {
    matches!(progress, Progress::Tried(Verdict::Fits(_)))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::ui::edit_view::grip_panel::{panel_radios, panel_words};
    use crate::ui::edit_view::grip_text::{
        PanelChoice, PanelModel, SolverSwitch, label_words, mm_amount,
    };
    use crate::ui::gtk_test::on_gtk_thread;
    use stave_app::session::Session;

    /// A copy of the side table built from a box (read, never written)
    /// in a temporary folder of its own.
    fn table(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("stave-gtk-grips-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/side_table_from_box/table.stave");
        std::fs::copy(source, folder.join("table.stave")).unwrap();
        folder.canonicalize().unwrap()
    }

    fn send(session: &mut Session, command: EditCommand) {
        let _ = session.apply(Command::Edit(command));
    }

    fn panel(session: &Session) -> PanelModel {
        let change = session.change().expect("a change is open");
        panel_model(
            change,
            |index| session.grips()?.grips().get(index).cloned(),
            false,
            None,
        )
        .expect("the grips' panel shows it")
    }

    fn apron_grips(folder: &Path, session: &mut Session) {
        let opened = stave_app::open(folder).unwrap();
        let model = opened.model.as_ref().unwrap();
        let apron = model
            .assemblies
            .iter()
            .find(|assembly| assembly.name == "apron_front")
            .unwrap()
            .id;
        send(
            session,
            EditCommand::ListGrips(GripSelection::Assembly(apron)),
        );
    }

    #[test]
    fn the_panel_says_what_a_drag_changes_and_lists_the_sizes_to_pick() {
        let folder = table("panel");
        let mut session = Session::new(folder.clone());
        apron_grips(&folder, &mut session);
        let grips = session.grips().unwrap();
        // The setback moves; the apron's length follows two plain params.
        assert_eq!(grip_look(&grips.grips()[1]), GripLook::Moves);
        assert_eq!(grip_look(&grips.grips()[3]), GripLook::Follows);
        assert_eq!(
            label_words(&grips.grips()[3]),
            "360mm · follows overall_width, legs.inset"
        );

        // A movable grip: the value it is moved to, what it was, the verdict.
        send(&mut session, EditCommand::BeginDrag(1));
        let waiting = panel(&session);
        assert_eq!(waiting.verdict.words, "Drag the number, or type a value");
        assert!(!waiting.can_apply);
        send(&mut session, EditCommand::DragTo(mm_amount(6.0).unwrap()));
        let moved = panel(&session);
        assert_eq!(moved.title, "apron_front.setback");
        assert_eq!(moved.value.as_deref(), Some("6mm"));
        assert_eq!(moved.was.as_deref(), Some("5mm"));
        assert_eq!(moved.verdict.words, "✓ It fits");
        assert!(moved.can_apply);

        // A size that follows two plain params: both, each with its value,
        // to pick from before anything is tried.
        send(&mut session, EditCommand::BeginDrag(3));
        send(&mut session, EditCommand::DragTo(mm_amount(380.0).unwrap()));
        let choosing = panel(&session);
        let words: Vec<&str> = choosing.choices.iter().map(|c| c.words.as_str()).collect();
        assert_eq!(
            words,
            ["overall_width 500mm to 520mm", "legs.inset 25mm to 15mm"]
        );
        assert_eq!(choosing.verdict.words, "Pick which size changes");
        // Each choice is a radio row: the name, and the value on the right.
        assert_eq!(choosing.choices[0].name, "overall_width");
        assert_eq!(choosing.choices[0].value, "500 → 520 mm");
        let about = choosing.about.as_ref().unwrap();
        assert_eq!(
            (about.label.as_str(), about.words.as_str()),
            ("Follows", "overall_width, legs.inset")
        );
        assert!(!choosing.can_apply);
        let width = choosing.choices[0].param;
        send(&mut session, EditCommand::PickParam(width));
        let picked = panel(&session);
        assert!(picked.choices[0].picked);
        assert_eq!(picked.changes, ["Changes overall_width 500mm to 520mm"]);
        assert!(picked.can_apply);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_scrub_of_forty_pixels_moves_a_size_forty_millimetres() {
        let folder = table("scrub");
        let mut session = Session::new(folder.clone());
        apron_grips(&folder, &mut session);
        let listed = session.grips().unwrap().grips()[1].clone();
        // The setback, 5mm: forty pixels right asks for 45mm, which the
        // session snaps and holds in its range.
        let wanted = stave_app::grip_values::scrubbed(grip_value(&listed), 40, false).unwrap();
        assert_eq!(amount_words(wanted), "45mm");
        let (target, _) = stave_app::grip_values::drag_target(&listed, wanted, false);
        send(&mut session, EditCommand::BeginDrag(1));
        send(&mut session, EditCommand::DragTo(target));
        let at = drag_target(session.change().unwrap(), 1).unwrap();
        assert_eq!(at.raw(), target.raw());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_sizes_card_shows_the_selections_own_sizes_and_folds_the_parts() {
        let folder = table("card");
        let mut session = Session::new(folder.clone());
        apron_grips(&folder, &mut session);
        let listed: Vec<(usize, ListedGrip)> = session
            .grips()
            .unwrap()
            .grips()
            .iter()
            .cloned()
            .enumerate()
            .collect();
        let count = listed.len();
        on_gtk_thread(move || {
            let (input, _receiver) = relm4::channel::<WindowInput>();
            let mut card = SizesCard::new(&input);
            card.show("apron_front", &listed, None);
            assert!(card.root.is_visible());
            // Every listed size has a row, by its index in the listing.
            assert_eq!(card.indices(), (0..count).collect::<Vec<_>>());
            assert_eq!(card.value_of(1).as_deref(), Some("5mm"));
            card.set_value(1, "6mm");
            assert_eq!(card.value_of(1).as_deref(), Some("6mm"));
            card.clear();
            assert!(!card.root.is_visible());
            assert!(card.indices().is_empty());
        });
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_typed_value_past_the_range_is_refused_before_the_stops_are_found() {
        let folder = table("typed");
        let mut session = Session::new(folder.clone());
        send(
            &mut session,
            EditCommand::ListGrips(GripSelection::Envelope),
        );
        let grips = session.grips().unwrap();
        let index = grips
            .grips()
            .iter()
            .position(|listed| listed.grip.label == "overall_width")
            .unwrap();
        let listed = grips.grips()[index].clone();
        assert!(matches!(
            listed.stops,
            stave_app::gripping::GripStops::NotSearched
        ));
        // The window refuses it, naming the range, before trying it.
        assert_eq!(
            typed_check(&listed, mm_amount(9000.0).unwrap()).unwrap_err(),
            "Can't be 9000mm: this grip moves it between 400mm and 600mm"
        );
        // Tried anyway, the session would write the end of the range: the
        // window sees another number than the one typed, and refuses it.
        send(
            &mut session,
            EditCommand::TypeGripValue {
                grip: index,
                value: mm_amount(9000.0).unwrap(),
            },
        );
        let written = typed_written(&listed, session.change().unwrap(), index).unwrap();
        assert_eq!(amount_words(written), "600mm");
        // A value inside the range is written as typed.
        send(&mut session, EditCommand::Cancel);
        let typed = mm_amount(550.0).unwrap();
        assert_eq!(typed_check(&listed, typed), Ok(()));
        send(
            &mut session,
            EditCommand::TypeGripValue {
                grip: index,
                value: typed,
            },
        );
        let written = typed_written(&listed, session.change().unwrap(), index).unwrap();
        assert_eq!(written.raw(), typed.raw());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_size_section_lists_the_sizes_as_radios() {
        on_gtk_thread(|| {
            let (sender, _receiver) = relm4::channel::<PanelAction>();
            let section = GripSection::new(&sender, |output| match output {
                GripPanelOutput::Action(action) => action,
            });
            let choice = |name: &str, value: &str, picked: bool| PanelChoice {
                param: ParamId::from_index(usize::from(picked)).unwrap(),
                words: format!("{name} {value}"),
                name: name.to_owned(),
                value: value.to_owned(),
                possible: true,
                picked,
            };
            let model = PanelModel {
                title: "case.base.length".to_owned(),
                value: Some("1000mm".to_owned()),
                was: Some("900mm".to_owned()),
                about: Some(super::super::grip_text::PanelAbout {
                    label: "Follows".to_owned(),
                    words: "overall_width".to_owned(),
                }),
                changes: Vec::new(),
                choices: vec![
                    choice("overall_width", "940 → 1040 mm", true),
                    choice("case.overhang", "20 → -30 mm", false),
                ],
                stop: Some("stops at 1200mm".to_owned()),
                verdict: super::super::grip_text::PanelVerdict {
                    class: "fits",
                    words: "✓ It fits".to_owned(),
                },
                can_apply: true,
            };
            section.show(&model, None);
            assert_eq!(
                panel_words(&section.panel),
                [
                    "case.base.length",
                    "1000mm",
                    "was 900mm",
                    "overall_width",
                    "stops at 1200mm"
                ]
            );
            assert_eq!(
                panel_radios(&section.panel),
                [
                    ("overall_width".to_owned(), true),
                    ("case.overhang".to_owned(), false)
                ]
            );
        });
    }

    #[test]
    fn a_rows_options_menu_is_freed_once_it_is_let_go() {
        on_gtk_thread(|| {
            let width = Ident::new("width", stave_lang::ast::Span::default()).unwrap();
            let switches = [SolverSwitch {
                param: width,
                value: mm_amount(100.0).unwrap(),
                solved: false,
            }];
            let (input, _receiver) = relm4::channel::<WindowInput>();
            let menu = super::super::grip_panel::switch_menu(&switches, move |ask, menu| {
                input.emit(switch_asked(0, ask, SwitchMenu::Row(menu)).into());
            });
            let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            holder.append(&menu);
            let weak = menu.downgrade();
            holder.remove(&menu);
            drop(menu);
            crate::ui::gtk_test::settle();
            // Its switch and button sit in its own popover: holding the
            // menu from their handlers would keep it alive for good.
            assert!(weak.upgrade().is_none(), "the menu is freed");
        });
    }
}
