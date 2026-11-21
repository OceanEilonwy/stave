//! Sizes in the Edit panel (`docs/gui.md`, "Sizes"; `docs/editing.md`,
//! "Grips"): the sizes of a selected assembly, of the envelope or of a
//! split's boundaries, one compact row each in the Edit panel's Sizes
//! card (`sizes.rs`); a row clicked opens the size editor in its place,
//! where the value is moved with a slider, stepped with buttons or typed,
//! and written with Apply. Nothing is drawn on the model and nothing is
//! written before Apply.
//!
//! ```text
//! selection / a click in a space ──ListGrips──▶ GripsChanged ──▶ the Sizes card's rows
//!                                               StopsJob on a worker ──▶ GripStopsFound ──▶ each slider's stops
//! click a row ──▶ the editor (nothing sent yet)
//! slide / step / type ──▶ the value shown at once, "Checking…"
//!     ──(a pause, or let go)──▶ BeginDrag(i), DragTo(value) ──▶ the verdict, what changes, the ghost
//! Apply (when it fits) ──Add──▶ written      Cancel or Escape ──Cancel──▶ nothing written
//! the switch (⋮) ──▶ MakeSolved / MakePlain ──▶ the change card ──Apply──▶ Add
//! ```
//!
//! The session decides everything: what a grip moves, where it snaps, what
//! stops it, the verdict and the text written; what a slider position or
//! a step is worth is `stave_app::grip_values`. A check takes a moment, so
//! the value is shown before it is checked, and a slider is checked only
//! once it pauses or is let go.

use stave_app::i18n::{fill_in, tr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use stave_app::change::{Change, ChangeRequest};
use stave_app::edit::{Picked, Verdict};
use stave_app::editing::{EditCommand, EditEvent, Progress, parse_value};
use stave_app::grip_values::{along, grip_bounds, nudged, position, slider_range};
use stave_app::gripping::{FoundStops, GripList, ListedGrip};
use stave_app::grips::{GripAction, GripSelection};
use stave_app::session::{Command, Event, Selection};
use stave_lang::ast::Ident;
use stave_model::ids::{AssemblyId, ParamId, PartId};
use stave_model::trace::Amount;
use stave_scene::Item;

use super::grip_panel::{
    GripOptions, GripPanelOutput, GripSection, PanelAction, SwitchAsk, SwitchMenu,
};
use super::grip_text::{
    GripLook, Held, amount_like, amount_words, arrow_words, grip_look, grip_value, held_words,
    panel_model, solver_switches, typed_check,
};
use super::size_slider::{SliderLook, Tone};
use super::sizes::{EditorView, SizesCard, short_name};
use crate::ui::update::editing::EditingInput;
use crate::ui::{Window, WindowInput};

/// How long a slider must rest before its value is checked.
const SLIDER_PAUSE: Duration = Duration::from_millis(120);

/// The sizes' card and section, and what only they need.
pub(in crate::ui) struct GripView {
    /// The size section of the change card, for a param made solved or
    /// plain; its actions come to the window as
    /// [`EditingInput::GripPanel`].
    panel: GripSection,
    /// The Sizes card of the Edit panel: a row per size, and the editor.
    pub(super) sizes: SizesCard,
    state: GripState,
}

/// What the grips keep between events.
#[derive(Default)]
struct GripState {
    /// What the sizes shown are of.
    shown_for: Option<GripSelection>,
    /// What the card's head names (`divider`), to shorten names by.
    title: String,
    /// Stops the running stops search.
    cancel: Option<Arc<AtomicBool>>,
    /// The size the editor is open on.
    open: Option<usize>,
    /// The value the editor holds, once moved.
    value: Option<Amount>,
    /// The value was held at an end of its range.
    held: Option<Held>,
    /// Why the value typed was refused.
    refusal: Option<String>,
    /// The value shown waits for its check.
    checking: bool,
    /// Which wait for a check is the latest.
    ticket: u64,
    /// The last value checked.
    last_sent: Option<Amount>,
    /// Apply was pressed while a check waited: written once it fits.
    apply_waiting: bool,
    /// A cancel the editor sent itself, which keeps it open.
    keep_open: bool,
    /// The size whose kind (solved or plain) a change is switching.
    kind_for: Option<usize>,
    /// The size just written, marked once its row is made again.
    flash: Option<usize>,
    /// A listing from the new design is on its way.
    relist_queued: bool,
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

/// `to` less `from`, signed, as `.stave` writes it: "+40mm", "−2.5mm";
/// empty when they are the same.
fn delta_words(from: Amount, to: Amount) -> String {
    let Some(by) = to.raw().checked_sub(from.raw()) else {
        return String::new();
    };
    if by == stave_lang::units::Rational::ZERO {
        return String::new();
    }
    let (sign, size) = if by.is_negative() {
        ("−", by.checked_neg().unwrap_or(by))
    } else {
        ("+", by)
    };
    format!("{sign}{}", amount_words(to.with_raw(size)))
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
            PanelAction::Type(text) => self.size_typed(&text),
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
        // While the text doesn't read the Sizes card lists nothing to
        // change; it is listed again once the text reads.
        let Some(selection) = wanted.filter(|_| self.gestures_allowed()) else {
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
                let busy = self.editing.grips.state.open.is_some();
                if !busy {
                    self.build_grip_marks();
                }
            }
            EditEvent::Started => self.grips_for_edit_start(),
            EditEvent::Committed => {
                self.reset_editor_state();
                self.editing.grips.sizes.close_editor();
                self.editing.grips.state.kind_for = None;
            }
            EditEvent::Cancelled => {
                let still_open = self.session.change().is_some_and(|change| {
                    matches!(
                        change.request(),
                        ChangeRequest::Grip { .. } | ChangeRequest::ParamKind { .. }
                    )
                });
                let (was_open, keep, region) = {
                    let state = &self.editing.grips.state;
                    (
                        state.open.is_some() || state.kind_for.is_some(),
                        state.keep_open,
                        matches!(
                            state.shown_for,
                            Some(GripSelection::Envelope | GripSelection::Split(_))
                        ),
                    )
                };
                // Dropped from elsewhere (Escape in the window, another
                // edit): the editor goes with it.
                if !still_open && !keep {
                    self.reset_editor_state();
                    self.editing.grips.sizes.close_editor();
                    self.editing.grips.state.kind_for = None;
                }
                // The menu of a space was let go (not a change of its
                // sizes): its sizes go with it.
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
            state.shown_for.is_none() || state.open.is_some()
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

    // ---- the rows ----

    fn clear_grip_marks(&mut self) {
        self.editing.grips.sizes.clear();
        self.reset_editor_state();
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
        self.editing.grips.state.title = title;
        if let Some(index) = self.editing.grips.state.flash.take() {
            self.editing.grips.sizes.flash(index);
        }
        self.update_edit_panel();
        self.refresh_sizes();
    }

    /// Shows each row's value, the open editor, and the change card's size
    /// section for a param made solved or plain.
    pub(in crate::ui) fn refresh_sizes(&mut self) {
        if let Some(list) = self.session.grips() {
            for (index, listed) in list.grips().iter().enumerate() {
                self.editing
                    .grips
                    .sizes
                    .set_row_value(index, &amount_words(grip_value(listed)));
            }
        }
        if let Some(view) = self.editor_view() {
            self.editing.grips.sizes.update_editor(&view);
        }
        self.show_grip_section();
    }

    /// What the open editor shows, from its value and the session.
    fn editor_view(&self) -> Option<EditorView> {
        let state = &self.editing.grips.state;
        let index = state.open?;
        let listed = self.listed_grip(index)?;
        let was = grip_value(&listed);
        let now = state.value.unwrap_or(was);
        let slider = slider_range(&listed);
        let bounds = grip_bounds(&listed);
        let change = self
            .session
            .change()
            .filter(|change| drag_index(change) == Some(index));
        let model =
            change.and_then(|change| panel_model(change, |at| self.listed_grip(at), false, None));
        let (tone, verdict) = if state.checking {
            (Tone::Checking, tr("Checking the design…"))
        } else if state.value.is_none() || state.value == Some(was) {
            (Tone::Waiting, tr("Drag, step or type a value"))
        } else {
            match &model {
                Some(model) => {
                    let tone = match model.verdict.class {
                        "fits" => Tone::Fits,
                        "refused" => Tone::Refused,
                        "checking" => Tone::Checking,
                        _ => Tone::Waiting,
                    };
                    (tone, model.verdict.words.clone())
                }
                None => (Tone::Waiting, tr("Drag, step or type a value")),
            }
        };
        let verdict = match state.held.and_then(|held| held_words(&listed, held)) {
            Some(stop) if tone != Tone::Checking => fill_in(
                &tr("{stop}. {verdict}"),
                &[("stop", &capitalised(&stop)), ("verdict", &verdict)],
            ),
            Some(_) | None => verdict,
        };
        let (tone, verdict) = if state.refusal.is_some() {
            (
                Tone::Refused,
                tr("Not tried: the value above can't be written"),
            )
        } else {
            (tone, verdict)
        };
        let moved = state.value.is_some_and(|value| value != was);
        Some(EditorView {
            index,
            name: short_name(&listed.grip.label, &state.title).to_owned(),
            was: amount_words(was),
            value: amount_words(now),
            delta: delta_words(was, now),
            slider: slider.and_then(|(low, high)| {
                Some(SliderLook {
                    stops: bounds.and_then(|(stop_low, stop_high)| {
                        Some((
                            position(low, high, stop_low)?,
                            position(low, high, stop_high)?,
                        ))
                    }),
                    was: position(low, high, was)?,
                    value: position(low, high, now)?,
                    tone,
                })
            }),
            low: bounds
                .or(slider)
                .map(|(low, _)| amount_words(low))
                .unwrap_or_default(),
            high: bounds
                .or(slider)
                .map(|(_, high)| amount_words(high))
                .unwrap_or_default(),
            changes: if moved {
                // The session lists what else changes (not while it is
                // checking: the list is the last value's then); a plain
                // number's own change is said first, and always.
                let mut lines = model
                    .as_ref()
                    .filter(|_| !state.checking)
                    .map(|model| model.changes.clone())
                    .unwrap_or_default();
                if matches!(listed.grip.action, GripAction::Moves(_)) {
                    lines.insert(
                        0,
                        format!("{}  {}", listed.grip.label, arrow_words(was, now)),
                    );
                }
                lines
            } else {
                Vec::new()
            },
            choices: model
                .as_ref()
                .map(|model| model.choices.clone())
                .unwrap_or_default(),
            verdict,
            tone,
            can_apply: moved
                && !state.checking
                && state.refusal.is_none()
                && model.as_ref().is_some_and(|model| model.can_apply),
            refusal: state.refusal.clone(),
            switches: if grip_look(&listed) == GripLook::Locked {
                Vec::new()
            } else {
                solver_switches(&listed)
            },
        })
    }

    // ---- the editor ----

    /// The listed grip at `index`.
    fn listed_grip(&self, index: usize) -> Option<ListedGrip> {
        self.session.grips()?.grips().get(index).cloned()
    }

    /// Forgets the editor's value and checks, leaving it as it is.
    fn reset_editor_state(&mut self) {
        let state = &mut self.editing.grips.state;
        state.open = None;
        state.value = None;
        state.held = None;
        state.refusal = None;
        state.checking = false;
        state.last_sent = None;
        state.apply_waiting = false;
        state.ticket = state.ticket.wrapping_add(1);
    }

    /// A row was clicked: the editor opens on it, nothing sent yet; one
    /// open on another size is closed, dropping its change.
    pub(in crate::ui) fn open_size(&mut self, index: usize) {
        if self.editing.grips.state.open == Some(index) {
            return;
        }
        self.close_size();
        self.editing.grips.state.open = Some(index);
        if let Some(view) = self.editor_view() {
            self.editing.grips.sizes.open_editor(&view);
        }
    }

    /// Cancel or Escape: the editor closes and its change is dropped.
    pub(in crate::ui) fn close_size(&mut self) {
        let open = self.editing.grips.state.open;
        let dragging = self
            .session
            .change()
            .is_some_and(|change| open.is_some() && drag_index(change) == open);
        self.reset_editor_state();
        self.editing.grips.sizes.close_editor();
        if dragging {
            self.dispatch(Command::Edit(EditCommand::Cancel));
        }
    }

    /// The value the editor's size is at now.
    fn size_now(&self) -> Option<Amount> {
        let state = &self.editing.grips.state;
        let listed = self.listed_grip(state.open?)?;
        Some(state.value.unwrap_or_else(|| grip_value(&listed)))
    }

    /// Moves the editor's value toward `wanted`, snapped and held in range
    /// as the session would, shown at once; checked after `wait`.
    fn set_size(&mut self, wanted: Amount, wait: Duration) {
        let Some(index) = self.editing.grips.state.open else {
            return;
        };
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        let (target, held) = stave_app::grip_values::drag_target(&listed, wanted, false);
        let ticket = {
            let state = &mut self.editing.grips.state;
            state.value = Some(target);
            state.held = held;
            state.refusal = None;
            state.checking = state.last_sent != Some(target);
            state.ticket = state.ticket.wrapping_add(1);
            state.ticket
        };
        self.refresh_sizes();
        if self.editing.grips.state.checking {
            let input = self.input();
            glib::timeout_add_local_once(wait, move || {
                input.emit(EditingInput::CheckSize(ticket).into());
            });
        }
    }

    /// The wait for a check is over: the latest value is tried, unless a
    /// newer wait has begun.
    pub(in crate::ui) fn check_size(&mut self, ticket: u64) {
        let (index, value) = {
            let state = &self.editing.grips.state;
            if state.ticket != ticket || !state.checking {
                return;
            }
            let (Some(index), Some(value)) = (state.open, state.value) else {
                return;
            };
            (index, value)
        };
        self.begin_grip_drag(index);
        self.dispatch(Command::Edit(EditCommand::DragTo(value)));
        let apply = {
            let state = &mut self.editing.grips.state;
            state.last_sent = Some(value);
            state.checking = false;
            std::mem::take(&mut state.apply_waiting)
        };
        self.refresh_sizes();
        if apply {
            self.apply_size();
        }
    }

    /// Starts a drag on `index` unless one is open on it (the `DragTo`
    /// that follows lists the sizes to pick from, for a size that follows
    /// several plain params).
    fn begin_grip_drag(&mut self, index: usize) {
        let going_on = self
            .session
            .change()
            .is_some_and(|change| drag_index(change) == Some(index));
        if going_on {
            return;
        }
        self.editing.grips.state.last_sent = None;
        self.dispatch(Command::Edit(EditCommand::BeginDrag(index)));
    }

    /// The slider moved to `at`; checked once it rests, or at once when
    /// it is let go.
    pub(in crate::ui) fn size_slid(&mut self, at: i32, released: bool) {
        let Some(listed) = self
            .editing
            .grips
            .state
            .open
            .and_then(|index| self.listed_grip(index))
        else {
            return;
        };
        let Some(wanted) = slider_range(&listed).and_then(|(low, high)| along(low, high, at))
        else {
            return;
        };
        let wait = if released {
            Duration::ZERO
        } else {
            SLIDER_PAUSE
        };
        self.set_size(wanted, wait);
    }

    /// A step button or an arrow key: `units` whole units (millimetres).
    pub(in crate::ui) fn size_nudged(&mut self, units: i32) {
        if let Some(wanted) = self.size_now().and_then(|now| nudged(now, units)) {
            self.set_size(wanted, Duration::ZERO);
        }
    }

    /// "Back to …": the value it had, the change dropped, the editor open.
    pub(in crate::ui) fn size_reset(&mut self) {
        let open = self.editing.grips.state.open;
        let dragging = self
            .session
            .change()
            .is_some_and(|change| open.is_some() && drag_index(change) == open);
        {
            let state = &mut self.editing.grips.state;
            state.value = None;
            state.held = None;
            state.refusal = None;
            state.checking = false;
            state.last_sent = None;
            state.ticket = state.ticket.wrapping_add(1);
            state.keep_open = dragging;
        }
        if dragging {
            self.dispatch(Command::Edit(EditCommand::Cancel));
        }
        self.editing.grips.state.keep_open = false;
        self.refresh_sizes();
    }

    /// Enter in the editor's field: what is typed is tried; Enter again on
    /// the value tried applies it. A value the session would write as
    /// another number (past an end, off the resolution) is refused first.
    pub(in crate::ui) fn size_typed(&mut self, text: &str) {
        let Some(index) = self.editing.grips.state.open else {
            return;
        };
        let Some(listed) = self.listed_grip(index) else {
            return;
        };
        let typed = amount_like(grip_value(&listed), text.trim())
            .and_then(|value| typed_check(&listed, value).map(|()| value));
        match typed {
            Err(why) => {
                self.editing.grips.state.refusal = Some(why);
                self.refresh_sizes();
            }
            Ok(value) => {
                let tried = {
                    let state = &self.editing.grips.state;
                    state.value == Some(value) && !state.checking
                };
                if tried {
                    self.apply_size();
                } else {
                    self.set_size(value, Duration::ZERO);
                }
            }
        }
    }

    /// Apply: writes the change when it fits; while a check waits, once it
    /// is done.
    pub(in crate::ui) fn apply_size(&mut self) {
        if self.editing.grips.state.checking {
            self.editing.grips.state.apply_waiting = true;
            return;
        }
        let open = self.editing.grips.state.open;
        let fits = self.session.change().is_some_and(|change| {
            open.is_some() && drag_index(change) == open && fits(change.progress())
        });
        if fits {
            self.editing.grips.state.flash = open;
            self.send_edit_later(EditCommand::Add);
        }
    }

    /// A pick from the sizes a follows grip could change, sent once the
    /// editor's own signal is done.
    fn pick_later(&mut self, param: ParamId) {
        self.send(EditingInput::Dispatch(Command::Edit(
            EditCommand::PickParam(param),
        )));
    }

    /// A grip's range switch turned off: the size is written plain again.
    pub(in crate::ui) fn grip_make_plain(&mut self, index: usize, param: Ident) {
        self.close_size();
        self.editing.grips.state.kind_for = Some(index);
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
                self.close_size();
                self.editing.grips.state.kind_for = Some(index);
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

    /// Draws the change card's size section for a param made solved or
    /// plain from a size's menu.
    fn show_grip_section(&mut self) {
        let session = &self.session;
        let grips = &self.editing.grips;
        let kind = session
            .change()
            .filter(|change| matches!(change.request(), ChangeRequest::ParamKind { .. }));
        let model = kind.and_then(|change| {
            panel_model(
                change,
                |index| session.grips()?.grips().get(index).cloned(),
                false,
                None,
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
        let options = grips.state.kind_for.and_then(|index| {
            let listed = session.grips()?.grips().get(index)?;
            let switches = solver_switches(listed);
            (grip_look(listed) != GripLook::Locked && !switches.is_empty())
                .then_some(GripOptions { index, switches })
        });
        grips.panel.show(&model, options);
        grips.panel.root.set_visible(true);
    }

    // ---- driving it without a pointer (scripts) ----

    /// The editor opened on `index` and its value moved to `value`, checked
    /// at once, not written.
    pub(in crate::ui) fn drag_grip_to(&mut self, index: usize, value: Amount) {
        self.open_size(index);
        self.set_size(value, Duration::ZERO);
        let ticket = self.editing.grips.state.ticket;
        self.check_size(ticket);
    }

    /// Apply, as the button.
    pub(in crate::ui) fn release_grip(&mut self) {
        let open = self.editing.grips.state.open;
        let fits = self.session.change().is_some_and(|change| {
            open.is_some() && drag_index(change) == open && fits(change.progress())
        });
        if fits {
            self.editing.grips.state.flash = open;
            self.dispatch(Command::Edit(EditCommand::Add));
        }
    }

    /// The editor opened on `index`, `text` typed and Enter pressed (and
    /// pressed again, to apply, when it fits).
    pub(in crate::ui) fn type_grip(&mut self, index: usize, text: &str) {
        self.open_size(index);
        self.editing.grips.sizes.type_text(text);
        self.size_typed(text);
        let ticket = self.editing.grips.state.ticket;
        self.check_size(ticket);
        if self.editing.grips.state.refusal.is_none() {
            self.release_grip();
        }
    }

    /// Opens grip `index`'s editor and its menu with the solver's switch.
    pub(in crate::ui) fn open_grip_menu(&mut self, index: usize) -> bool {
        self.open_size(index);
        self.editing.grips.sizes.open_menu()
    }

    /// Marks a change of a param's kind as started from grip `index`.
    pub(in crate::ui) fn grip_change_opened(&mut self, index: usize) {
        self.editing.grips.state.kind_for = Some(index);
    }

    /// The listed grip numbered `index`, for a script.
    pub(in crate::ui) fn grip_at(&self, index: usize) -> Option<ListedGrip> {
        self.listed_grip(index)
    }
}

/// `text` with its first letter capitalised.
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

fn drag_index(change: &Change) -> Option<usize> {
    match change.request() {
        ChangeRequest::Grip { index } => Some(*index),
        _ => None,
    }
}

/// Where a drag on `index` was last moved to.
#[cfg(test)]
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
    use crate::ui::edit_view::size_slider::Tone;
    use crate::ui::gtk_test::on_gtk_thread;
    use stave_app::grip_values::SLIDER_STEPS;
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
        assert_eq!(waiting.verdict.words, "Drag, step or type a value");
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
    fn the_sizes_card_lists_rows_and_opens_the_editor_in_place() {
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
            // The editor opens on a row, showing what it is handed.
            let view = EditorView {
                index: 1,
                name: "setback".to_owned(),
                was: "5mm".to_owned(),
                value: "6mm".to_owned(),
                delta: "+1mm".to_owned(),
                verdict: "It fits".to_owned(),
                tone: Tone::Fits,
                can_apply: true,
                ..EditorView::default()
            };
            card.open_editor(&view);
            assert_eq!(card.open(), Some(1));
            crate::ui::gtk_test::settle();
            assert_eq!(
                card.editor_words(),
                (
                    "6mm".to_owned(),
                    "+1mm".to_owned(),
                    "It fits".to_owned(),
                    true
                )
            );
            card.close_editor();
            assert_eq!(card.open(), None);
            card.clear();
            assert!(!card.root.is_visible());
            assert!(card.indices().is_empty());
        });
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_value_moved_by_its_slider_or_buttons_is_snapped_and_held_by_the_session_rules() {
        let folder = table("slider");
        let mut session = Session::new(folder.clone());
        apron_grips(&folder, &mut session);
        let listed = session.grips().unwrap().grips()[1].clone();
        // The setback, 5mm: +10 asks for 15mm.
        let wanted = nudged(grip_value(&listed), 10).unwrap();
        let (target, _) = stave_app::grip_values::drag_target(&listed, wanted, false);
        assert_eq!(amount_words(target), "15mm");
        assert_eq!(delta_words(grip_value(&listed), target), "+10mm");
        // The slider's far right end asks for more than the range allows:
        // held at the high end.
        let (low, high) = slider_range(&listed).unwrap();
        let (_, held) = stave_app::grip_values::drag_target(
            &listed,
            along(low, high, SLIDER_STEPS).unwrap(),
            false,
        );
        assert_eq!(held, Some(Held::Above));
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
        // A value inside the range is tried as typed.
        let typed = mm_amount(550.0).unwrap();
        assert_eq!(typed_check(&listed, typed), Ok(()));
        send(
            &mut session,
            EditCommand::TypeGripValue {
                grip: index,
                value: typed,
            },
        );
        let written = drag_target(session.change().unwrap(), index).unwrap();
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
