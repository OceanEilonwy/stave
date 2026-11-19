//! Editing in the 3D view, wired to the session (`docs/editing.md`,
//! `docs/gui.md` "Editing in the 3D view"): the pointer lights the space
//! under it, a click in a space or in the gap between two facing parts
//! opens the menu for it, the part's card offers to add onto the face
//! clicked, and the menu and the card send the session's commands. The
//! session decides everything; this file only turns what people do into
//! [`EditCommand`]s and draws what the session holds after each
//! [`EditEvent`].
//!
//! ```text
//! pointer ──pick_target──▶ Region ─▶ lit (and what covers it see-through)
//! click   ──pick_target──▶ Region / facing pair ─▶ EditCommand::Start ─▶ menu
//! menu entry ─▶ Choose ─▶ card ─▶ Set / Bind (next click) / Rename / ApplyFix
//!                              └▶ Apply (EditCommand::Add) ─▶ Committed: the new design, the item selected
//!                              └▶ Cancel or Escape ─▶ nothing written
//! chosen candidate tried ─▶ its ghost (Ghosts over the base), tinted by verdict
//! ```
//!
//! What a widget reports is sent a moment later ([`Window::send_edit_later`]),
//! never from inside the widget's own signal: the card is drawn again from
//! the session on every event, and its name field reports when it is taken
//! away.

use stave_app::i18n::{fill_in, tr};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use stave_app::edit::{
    Applicability, Declaration, Names, PartFace, Picked, PlaceSlot, Problem, Refusal, Verdict,
};
use stave_app::editing::{
    Edit, EditCommand, EditEvent, Progress, parse_name, parse_value, picked_from_facing,
    picked_from_part_face,
};
use stave_app::option_fits::{FoundFit, OptionsTicket};
use stave_app::session::{Command, Editor, Selection, Session};
use stave_lang::ast::Expr;
use stave_lang::{Code, Diagnostic, Severity};
use stave_model::ids::AssemblyId;
use stave_model::solved::RegionId;
use stave_scene::{
    Aabb, BuildOptions, Extent, Facing, GhostVerdict, Ghosts, Look, NodeId, Orbit, Posed, Ray,
    SceneGraph, Subject, Target,
};

use super::edit_card::{
    CardAction, CardModel, CardOffers, CardSlots, CardView, EditCardComponent, EditCardInput,
    EditCardOutput, Words,
};
use super::edit_menu::{
    Checked, EditMenuComponent, EditMenuInput, EditMenuOutput, MenuEntry, MenuItem,
};
use super::edit_panel::EditPanel;
use super::edit_writer::BufferWriter;
use super::option_search::OptionSearch;
use super::update::editing::EditingInput;
use super::{EDGE, Window, WindowInput};
use crate::render::{Backdrop, Frame, Ticket};
use crate::ui::update::Unprinted;
use relm4::{Component, ComponentController, Controller};

/// How long one step of the menu's check runs before the window draws
/// what it found so far; a step that finds nothing new gets twice as long.
const MENU_STEP: Duration = Duration::from_millis(120);
/// The longest a step of the menu's check may run.
const MENU_STEP_LIMIT: Duration = Duration::from_secs(8);
/// How long the card shows "Checking" before a command that tries the
/// candidate again runs, so the spinner is drawn first.
const CHECKING_DELAY: Duration = Duration::from_millis(40);
/// How strongly a ghost takes its verdict's colour.
const GHOST_TINT: f32 = 0.62;

/// The editing widgets (the menu at the click, the cards in the Edit
/// panel, the labels on the model), and what only they need.
pub(super) struct EditView {
    /// The menu for a selection, a pop-up at the click.
    pub(super) menu: Controller<EditMenuComponent>,
    /// The card for a chosen candidate, in the Edit panel.
    pub(super) card: Controller<EditCardComponent>,
    /// The card's boxes the window fills (the repeat controls, the
    /// Pending section).
    pub(super) card_slots: CardSlots,
    /// What the card shown last offers (its short lists, the put-back
    /// button), from the view the window drew it with; none while it is
    /// hidden.
    card_offers: Option<CardOffers>,
    /// The setting whose short list is open on the card, as the card last
    /// said ([`EditCardOutput::OptionsOpen`]) or the window opened it.
    pub(super) card_open_options: Option<String>,
    /// The Edit panel: the card in progress, or how to start.
    pub(super) panel: EditPanel,
    /// Whether the panel showed a card when last drawn.
    panel_shown: bool,
    /// The search over a setting's options.
    pub(super) search: Rc<OptionSearch>,
    /// On the part's card: add onto the face that was clicked.
    face_button: gtk::Button,
    /// Whether the editor shows, for the writer (a commit opens the file
    /// in a tab when it does).
    editor_open: Rc<Cell<bool>>,
    /// The grips of the selection, on the model.
    grips: grips::GripView,
    state: EditState,
    // The changes of the design (lane W2b): their card, the item's tools,
    // hold, repeat handles, the editor's pending lines, the finder's verbs.
    changes: changes::Changes,
    tools: item_tools::ItemTools,
    hold: hold::Hold,
    repeats: repeat_handles::Repeats,
    pending: editor_link::PendingPanel,
    pending_marked: Option<std::path::PathBuf>,
    verbs: Vec<verbs::Run>,
    /// Scripts send each command at once, in order.
    immediate: bool,
    /// The check of the chosen candidate's options running on a worker
    /// thread: its cancel flag and the candidate it is for.
    options: Option<(Arc<AtomicBool>, OptionsTicket)>,
    /// The fixture's menu or card (`--edit-fixture`), kept while the
    /// window is open.
    pub(super) showcase: Option<super::edit_demo::Shown>,
}

/// What the editing view keeps between events.
#[derive(Default)]
struct EditState {
    /// The space lit under the pointer.
    hover: Option<RegionId>,
    /// The face last clicked on the selected part.
    face: Option<PartFace>,
    /// Where the menu opens, in the 3D view's logical pixels.
    point: Option<(f64, f64)>,
    /// The chosen candidate's ghost, while it has one.
    ghost: Option<GhostView>,
    /// Counts edits, so a menu check meant for an earlier one stops.
    generation: u64,
    /// A command that tries the candidate again is waiting to run.
    checking: bool,
    /// The name of the item being added, to select once it is written.
    select_after: Option<String>,
    /// A space lit by name until an edit starts (a new project's
    /// envelope).
    first_space: Option<String>,
    /// What a link in the Inspector or the Edit panel names, lit while
    /// the pointer is on it (`links.rs`).
    link_hover: Option<links::Hovered>,
}

/// A candidate's ghost: its scene, posed at rest.
struct GhostView {
    ghosts: Ghosts,
    posed: Posed,
    /// The renderer holds this scene.
    uploaded: bool,
}

impl EditView {
    /// The widgets, hidden until an edit starts. The menu's and the
    /// card's outputs go to the window's input as [`EditingInput`]s.
    pub(super) fn new(input: &relm4::Sender<WindowInput>) -> EditView {
        let menu = EditMenuComponent::builder()
            .launch(())
            .forward(input, menu_heard);
        let root = menu.widget();
        root.set_halign(gtk::Align::Start);
        root.set_valign(gtk::Align::Start);
        root.set_visible(false);
        let card_slots = CardSlots::new();
        let card = EditCardComponent::builder()
            .launch(card_slots.clone())
            .forward(input, card_heard);
        card.widget().set_visible(false);
        let face_button = gtk::Button::builder()
            .css_classes(["stave-edit-face", "pill"])
            .margin_start(16)
            .margin_end(16)
            .margin_bottom(16)
            .visible(false)
            .build();
        let changes = changes::Changes::new();
        let hold = hold::Hold::new();
        let panel = EditPanel::new(&[
            card.widget().upcast_ref(),
            changes.card.root.upcast_ref(),
            hold.card.root.upcast_ref(),
        ]);
        EditView {
            menu,
            card,
            card_slots,
            card_offers: None,
            card_open_options: None,
            panel,
            panel_shown: false,
            search: OptionSearch::new(),
            face_button,
            editor_open: Rc::new(Cell::new(false)),
            grips: grips::GripView::new(),
            state: EditState::default(),
            changes,
            tools: item_tools::ItemTools::new(),
            hold,
            repeats: repeat_handles::Repeats::new(),
            pending: editor_link::PendingPanel::new(),
            pending_marked: None,
            verbs: Vec::new(),
            immediate: false,
            options: None,
            showcase: None,
        }
    }
}

/// What the menu tells the window, as the window's input.
fn menu_heard(output: EditMenuOutput) -> WindowInput {
    match output {
        EditMenuOutput::Chosen(declaration) => EditingInput::MenuChosen(declaration),
        EditMenuOutput::Drawn => EditingInput::MenuDrawn,
    }
    .into()
}

/// What the card tells the window, as the window's input.
fn card_heard(output: EditCardOutput) -> WindowInput {
    match output {
        EditCardOutput::Action(action) => EditingInput::CardAction(action),
        EditCardOutput::OptionsOpen(setting) => EditingInput::CardOptionsOpen(setting),
        EditCardOutput::Link(target) => EditingInput::LinkClicked(target),
        EditCardOutput::Hover(target) => EditingInput::LinkHovered(target),
    }
    .into()
}

/// The words for the ids and expressions a candidate holds, from the
/// model of the design the edit is tried on.
pub(super) struct SessionWords<'m> {
    names: Option<Names<'m>>,
    design: Option<&'m stave_model::solved::SolvedDesign>,
}

impl<'m> SessionWords<'m> {
    /// Words from the edit's base model.
    pub(super) fn of(edit: &'m Edit) -> SessionWords<'m> {
        SessionWords {
            names: edit.base().model.as_ref().map(Names::new),
            design: edit.base().design.as_ref(),
        }
    }
}

impl Words for SessionWords<'_> {
    fn expression(&self, expression: &Expr) -> String {
        stave_app::edit::value_text(expression)
    }

    fn assembly(&self, assembly: AssemblyId) -> String {
        self.names
            .and_then(|names| names.assembly(assembly))
            .map_or_else(|| tr("another assembly"), str::to_owned)
    }

    fn place(&self, slot: &PlaceSlot) -> Option<String> {
        self.names?.slot(slot)
    }

    fn links(&self, text: &str) -> Vec<stave_app::inspect::NameLink> {
        self.design
            .map(|design| stave_app::inspect::place_links(design, text))
            .unwrap_or_default()
    }
}

/// The menu's entries as the session has them: each entry's
/// applicability is final once worked out; one not checked yet shows as
/// being checked.
pub(super) fn menu_items(edit: &Edit) -> Vec<MenuItem<'_>> {
    edit.menu()
        .iter()
        .map(|candidate| MenuItem {
            candidate,
            checked: Checked::Done,
        })
        .collect()
}

/// How many entries of the menu are not checked yet.
pub(super) fn unchecked(edit: &Edit) -> usize {
    edit.menu()
        .iter()
        .filter(|candidate| candidate.applicability == Applicability::NotChecked)
        .count()
}

/// The index in the session's menu of the entry for a declaration.
pub(super) fn entry_index(edit: &Edit, declaration: Declaration) -> Option<usize> {
    edit.menu()
        .iter()
        .position(|candidate| candidate.declaration == declaration)
}

/// The verdict the card shows: none while something is still to choose,
/// Checking while a command is waiting to try it again, else the try's.
pub(super) fn card_verdict(progress: &Progress, checking: bool) -> Option<Verdict> {
    if checking {
        return Some(Verdict::Checking);
    }
    match progress {
        Progress::NeedsChoice(_) => None,
        Progress::Tried(verdict) => Some(verdict.clone()),
        Progress::Unwritable(reason) => {
            let diagnostic = Diagnostic {
                severity: Severity::Error,
                code: Code::NotSupported,
                message: reason.clone(),
                labels: Vec::new(),
                notes: Vec::new(),
                merged: Vec::new(),
                help: None,
                fixes: Vec::new(),
                cause: None,
            };
            let problem = Problem::new(diagnostic, Vec::new()).ok()?;
            Refusal::new(vec![problem]).ok().map(Verdict::Refused)
        }
    }
}

/// The command a card action asks for, or what is wrong with what was
/// typed, in words.
pub(super) fn command_for(action: CardAction) -> Result<EditCommand, String> {
    match action {
        CardAction::Rename(text) => parse_name(&text).map(EditCommand::Rename).map_err(|error| {
            fill_in(
                &tr("'{text}' can't be a name: {reason}"),
                &[("text", &text), ("reason", &error.0)],
            )
        }),
        CardAction::Choose { param, text } => parse_value(&text)
            .map(|value| EditCommand::Set { name: param, value })
            .map_err(|error| {
                fill_in(
                    &tr("'{text}' isn't a value Stave can read: {reason}"),
                    &[("text", &text), ("reason", &error.0)],
                )
            }),
        CardAction::Fix { problem, fix } => Ok(EditCommand::ApplyFix { problem, fix }),
        CardAction::UseDefault { param } => Ok(EditCommand::UseDefault(param)),
        CardAction::Search { param } => Err(format!(
            "the search over {} is opened by the window, not sent",
            param.as_str()
        )),
        CardAction::PutBack => Ok(EditCommand::PutBack),
        CardAction::Add => Ok(EditCommand::Add),
        CardAction::Cancel => Ok(EditCommand::Cancel),
    }
}

/// `true` for a command that tries the candidate again (the card shows
/// Checking until it has).
fn tries_again(command: &EditCommand) -> bool {
    matches!(
        command,
        EditCommand::Choose(_)
            | EditCommand::Bind(_)
            | EditCommand::Set { .. }
            | EditCommand::PutBack
            | EditCommand::UseDefault(_)
            | EditCommand::Rename(_)
            | EditCommand::Repeat(_)
            | EditCommand::ApplyFix { .. }
            | EditCommand::RepeatItem { .. }
            | EditCommand::MoveShares { .. }
            | EditCommand::Mirror(_)
            | EditCommand::MirrorSplit { .. }
            | EditCommand::Hold(_)
            | EditCommand::Remove(_)
            | EditCommand::IncludeDependants
            | EditCommand::RenameItem { .. }
    )
}

/// What [`draw_edit`] showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Drawn {
    /// No edit is in progress: neither shows.
    Nothing,
    /// The menu, nothing chosen yet.
    Menu,
    /// The card, offering this.
    Card(CardOffers),
}

impl Drawn {
    /// `true` while an edit is in progress.
    pub(super) fn is_open(&self) -> bool {
        match self {
            Drawn::Nothing => false,
            Drawn::Menu | Drawn::Card(_) => true,
        }
    }
}

/// Draws the edit in progress: the menu while nothing is chosen, the
/// card once something is, neither without an edit. Which shows changes
/// at once; what they show is sent to them, drawn when the main loop next
/// runs. Returns what it showed, with what the card offers.
pub(super) fn draw_edit(
    menu: &Controller<EditMenuComponent>,
    card: &Controller<EditCardComponent>,
    session: &Session,
    checking: bool,
) -> Drawn {
    let Some(edit) = session.edit() else {
        menu.widget().set_visible(false);
        card.widget().set_visible(false);
        return Drawn::Nothing;
    };
    let heading = edit.picked().heading();
    match edit.chosen() {
        None => {
            // Sending fails only once the menu has shut down, with the
            // window: nothing is left to draw.
            let _ = menu.sender().send(EditMenuInput::Show {
                heading,
                items: MenuEntry::list(&menu_items(edit)),
            });
            menu.widget().set_visible(true);
            card.widget().set_visible(false);
            Drawn::Menu
        }
        Some(chosen) => {
            let candidate = chosen.candidate();
            let verdict = card_verdict(chosen.progress(), checking);
            let name = chosen
                .item_name()
                .or(candidate.generated_name.as_ref())
                .map(|name| name.as_str().to_owned())
                .unwrap_or_default();
            let applied: Vec<String> = chosen
                .fixes()
                .iter()
                .map(|fix| fix.label().to_owned())
                .collect();
            let view = CardView::new(
                &CardModel {
                    applied: &applied,
                    blame: chosen.blame(),
                    heading,
                    candidate,
                    name: &name,
                    verdict: verdict.as_ref(),
                },
                &SessionWords::of(edit),
            );
            let offers = view.offers();
            let _ = card.sender().send(EditCardInput::Show(Box::new(view)));
            menu.widget().set_visible(false);
            card.widget().set_visible(true);
            Drawn::Card(offers)
        }
    }
}

/// The colour a ghost is tinted with, from its verdict.
fn ghost_tint(verdict: GhostVerdict) -> [f32; 4] {
    let [r, g, b] = verdict.tint();
    [r, g, b, GHOST_TINT]
}

/// The ghost's verdict for where a chosen candidate stands; `None` when
/// it has not been tried.
fn ghost_verdict(progress: &Progress) -> Option<GhostVerdict> {
    match progress {
        Progress::Tried(Verdict::Checking) => Some(GhostVerdict::Checking),
        Progress::Tried(Verdict::Fits(_)) => Some(GhostVerdict::Fits),
        Progress::Tried(Verdict::Refused(_)) => Some(GhostVerdict::Refused),
        Progress::NeedsChoice(_) | Progress::Unwritable(_) => None,
    }
}

/// The nodes of `graph` that cover a region from the camera, found by its
/// name (a ghost's scene numbers its regions afresh).
fn covering_region(graph: &SceneGraph, posed: &Posed, orbit: &Orbit, name: &str) -> Vec<NodeId> {
    graph
        .regions()
        .iter()
        .find(|region| region.name == name)
        .map(|region| graph.covering(posed, orbit, Subject::Region(region.id)))
        .unwrap_or_default()
}

/// What the editor adds to a frame of the design: the spaces lit and the
/// nodes drawn see-through because they cover the space worked on.
#[derive(Debug, Default)]
pub(super) struct EditMarks {
    /// The spaces lit, posed.
    pub(super) lit: Vec<Aabb>,
    /// The nodes in front of the space worked on.
    pub(super) covering: Vec<NodeId>,
}

impl Window {
    /// Puts the editing widgets over the 3D view and connects them.
    pub(super) fn connect_editing(&mut self) {
        let editing = &self.editing;
        let design = &self.widgets.design;
        design.add_overlay(editing.menu.widget());
        self.widgets.edit_holder.append(&editing.panel.root);
        self.widgets.sheet.add_overlay(&editing.search.root);
        self.connect_option_search();
        self.widgets.details.append(&editing.face_button);
        self.install_edit_writer();
        self.connect_changes();
        self.connect_item_tools();
        self.connect_hold();
        self.connect_repeats();
        self.connect_editor_link();

        // The menu's and the card's outputs were forwarded to the window
        // when they were launched (`EditView::new`).
        let input = self.input();
        self.editing
            .face_button
            .connect_clicked(move |_| input.emit(EditingInput::EditFace.into()));

        // The pointer lights the space under it.
        let motion = gtk::EventControllerMotion::new();
        let input = self.input();
        motion.connect_motion(move |_, x, y| {
            input.emit(EditingInput::PointerAt(Some((x, y))).into());
        });
        let input = self.input();
        motion.connect_leave(move |_| input.emit(EditingInput::PointerAt(None).into()));
        self.widgets.viewport.add_controller(motion);
        self.connect_grips();
    }

    /// Add writes through the editor's buffers.
    fn install_edit_writer(&mut self) {
        self.session.set_edit_writer(Box::new(BufferWriter::new(
            &self.widgets.files,
            self.editing.editor_open.clone(),
        )));
    }

    /// Another folder is open: a fresh session, nothing being edited.
    pub(super) fn editing_folder_changed(&mut self) {
        self.install_edit_writer();
        {
            let state = &mut self.editing.state;
            let first_space = state.first_space.take();
            *state = EditState {
                generation: state.generation.wrapping_add(1),
                ..EditState::default()
            };
            state.first_space = first_space;
        }
        self.refresh_edit();
    }

    /// Lights a space by name until an edit starts: a new project's
    /// envelope, the first space to click.
    pub(super) fn light_first_space(&mut self, name: &str) {
        self.editing.state.first_space = Some(name.to_owned());
    }

    /// `true` while an edit (or a change) is in progress.
    pub(super) fn edit_open(&self) -> bool {
        let session = &self.session;
        session.edit().is_some() || session.change().is_some()
    }

    /// Escape: drops the edit in progress. `true` when there was one.
    pub(super) fn cancel_edit(&mut self) -> bool {
        if self.cancel_hold() {
            return true;
        }
        if !self.edit_open() {
            return false;
        }
        self.dispatch(Command::Edit(EditCommand::Cancel));
        true
    }

    // ---- what the pointer is on ----

    /// The ray through a point of the 3D view.
    fn ray_at(&self, x: f64, y: f64) -> Option<Ray> {
        let state = &self.state;
        let viewport = &self.widgets.viewport;
        state.orbit.map(|orbit| {
            orbit.ray(
                crate::convert::f32_of(x),
                crate::convert::f32_of(y),
                crate::convert::f32_of_int(viewport.width()),
                crate::convert::f32_of_int(viewport.height()),
            )
        })
    }

    /// What a point of the 3D view is on: a part's face or a space, and
    /// the two parts facing across the gap there.
    fn pointed_at(&self, x: f64, y: f64) -> (Option<Target>, Option<Facing>) {
        let Some(ray) = self.ray_at(x, y) else {
            return (None, None);
        };
        let session = &self.session;
        let state = &self.state;
        let (Some((_, graph)), Some(posed)) = (session.drawable(), state.posed.as_ref()) else {
            return (None, None);
        };
        let target = graph.pick_target(posed, ray);
        let facing = match target {
            None => graph.facing(posed, ray),
            Some(_) => None,
        };
        (target, facing)
    }

    /// What the chosen candidate still waits to have pointed at; the next
    /// click binds it when anything is.
    pub(super) fn wanted(&self) -> pointing::Wanted {
        pointing::Wanted::of(self.session.edit())
    }

    /// What a click at a point of the 3D view binds, for what the card
    /// asks to be pointed at.
    pub(super) fn binding_picked(
        &self,
        x: f64,
        y: f64,
        wanted: pointing::Wanted,
    ) -> Option<Picked> {
        let ray = self.ray_at(x, y)?;
        let session = &self.session;
        let state = &self.state;
        let (_, graph) = session.drawable()?;
        pointing::binding_pick(graph, state.posed.as_ref()?, ray, wanted)
    }

    /// The pointer moved over the 3D view (or left it: `None`): lights the
    /// space under it while nothing is chosen, or while the card asks for
    /// a place to point at.
    pub(in crate::ui) fn edit_hover(&mut self, point: Option<(f64, f64)>) {
        let hover = match point {
            Some((x, y))
                if self.state.focus.is_none()
                    && (self.session.edit().is_none() || self.wanted().region) =>
            {
                match self.pointed_at(x, y).0 {
                    Some(Target::Region(hit)) => Some(hit.region),
                    Some(Target::Node(_)) | None => None,
                }
            }
            Some(_) | None => None,
        };
        let changed = {
            let state = &mut self.editing.state;
            let changed = state.hover != hover;
            state.hover = hover;
            changed
        };
        if changed {
            self.request_render();
        }
    }

    /// A click in the 3D view, for editing first: a space or a gap opens
    /// the menu for it, and while the card asks for a place the click
    /// binds it. A click on a part is left to the selection (its face is
    /// kept for the card's "Add onto" button), dropping any edit. `true`
    /// when the click was used here.
    pub(super) fn edit_click(&mut self, x: f64, y: f64) -> bool {
        if self.state.focus.is_some() {
            return false;
        }
        if self.hold_click(x, y) {
            return true;
        }
        let wanted = self.wanted();
        if wanted.any() {
            // A part's face is wanted even where the click goes through a
            // space to reach it.
            if let Some(picked) = self.binding_picked(x, y, wanted) {
                self.send_edit_later(EditCommand::Bind(picked));
            }
            return true;
        }
        let (target, facing) = self.pointed_at(x, y);
        let picked = match target {
            Some(Target::Region(hit)) => Some(Picked::Region(hit.region)),
            Some(Target::Node(hit)) => hit.part_face().map(picked_from_part_face),
            None => facing.as_ref().and_then(picked_from_facing),
        };
        match (target, picked) {
            (Some(Target::Region(_)), Some(_)) | (None, Some(_)) if !self.gestures_allowed() => {
                self.edit_toast(&self.parse_block_line().unwrap_or_default());
                true
            }
            (Some(Target::Region(_)), Some(picked)) | (None, Some(picked)) => {
                self.start_edit(picked, (x, y));
                true
            }
            (Some(Target::Node(_)), picked) => {
                self.editing.state.face = match picked {
                    Some(Picked::Face(face)) => Some(face),
                    Some(Picked::Region(_) | Picked::Between(_) | Picked::Group(_)) | None => None,
                };
                self.cancel_edit();
                false
            }
            (Some(Target::Region(_)) | None, None) => {
                self.cancel_edit();
                false
            }
        }
    }

    /// Starts an edit from a selection: the menu opens near `point`.
    pub(super) fn start_edit(&mut self, picked: Picked, point: (f64, f64)) {
        let keeps_selection = matches!(picked, Picked::Face(_));
        {
            let session = &mut self.session;
            session.set_edit_files(Arc::new(self.widgets.files.edit_files()));
        }
        self.editing.state.point = Some(point);
        if !keeps_selection && self.session.selection().is_some() {
            self.dispatch(Command::Select(None));
        }
        self.dispatch(Command::Edit(EditCommand::Start(picked)));
    }

    /// The part's card's button: the menu for the face clicked on it.
    pub(in crate::ui) fn edit_face(&mut self) {
        let Some(face) = self.editing.state.face else {
            return;
        };
        let point = self
            .screen_point(&self.selected_nodes())
            .map_or((200.0, 200.0), |(x, y)| (f64::from(x), f64::from(y)));
        self.start_edit(Picked::Face(face), point);
    }

    /// Shows the part's "Add onto" button when the selected part is the
    /// one whose face was clicked and nothing is being edited.
    pub(super) fn update_face_button(&mut self) {
        let face = self.editing.state.face;
        let selected = match self.session.selection() {
            Some(Selection::Part(part)) => Some(*part),
            _ => None,
        };
        let shown = match (face, selected) {
            (Some(face), Some(part)) if face.part == part && !self.edit_open() => Some(face),
            _ => None,
        };
        let button = &self.editing.face_button;
        if let Some(face) = shown {
            button.set_label(&fill_in(
                &tr("Add something onto its {face} face…"),
                &[("face", &face.face.word())],
            ));
            button.set_tooltip_text(Some(&tr("List what can go on the face you clicked")));
        }
        button.set_visible(shown.is_some());
        // A new project's box is the first space to click.
        let waiting = self.editing.state.first_space.is_some()
            || self
                .session
                .drawable()
                .is_some_and(|(_, graph)| pointing::only_spaces(graph));
        if waiting && !self.edit_open() && self.session.selection().is_none() {
            self.show_hint(&tr("Click inside the box to add the first piece"));
        }
        self.update_item_tools();
        self.widgets.tree.set_placeholder(
            waiting
                .then(|| tr("No parts yet. Click inside the box to add the first piece."))
                .as_deref(),
        );
        self.show_parse_block_line();
    }

    // ---- commands ----

    /// What the card reports, as the session's command.
    pub(in crate::ui) fn card_action(&mut self, action: CardAction) {
        if let CardAction::Search { param } = &action {
            self.open_option_search(param.as_str(), "");
            return;
        }
        match command_for(action) {
            Ok(command) => self.send_edit_later(command),
            Err(words) => self.edit_toast(&words),
        }
    }

    fn edit_toast(&self, words: &str) {
        self.widgets
            .toasts
            .add_toast(adw::Toast::builder().title(words).timeout(4).build());
    }

    /// Sends an edit command once the widget that asked has finished its
    /// own signal; a command that tries the candidate again first shows
    /// Checking (the card's spinner and the ghost's grey).
    pub(super) fn send_edit_later(&mut self, command: EditCommand) {
        if self.editing.immediate {
            self.send_edit(command);
            return;
        }
        if tries_again(&command) {
            self.show_checking();
        }
        let input = self.input();
        glib::timeout_add_local_once(CHECKING_DELAY, move || {
            input.emit(EditingInput::SendEdit(command).into());
        });
    }

    /// Chooses a menu entry once the menu's own signal is done. The entry
    /// is looked up by its declaration when the command is sent, not when
    /// it was clicked: a step of the menu's check may sort the menu in
    /// between (greyed entries go last in their group), moving indexes.
    pub(super) fn choose_later(&mut self, declaration: Declaration) {
        if self.editing.immediate {
            self.choose_now(declaration);
            return;
        }
        self.show_checking();
        let input = self.input();
        glib::timeout_add_local_once(CHECKING_DELAY, move || {
            input.emit(EditingInput::ChooseNow(declaration).into());
        });
    }

    /// Chooses the menu entry for a declaration now, wherever the menu
    /// holds it; an entry that is gone chooses nothing.
    pub(super) fn choose_now(&mut self, declaration: Declaration) {
        let index = self
            .session
            .edit()
            .and_then(|edit| entry_index(edit, declaration));
        match index {
            Some(index) => self.send_edit(EditCommand::Choose(index)),
            None => {
                self.editing.state.checking = false;
                self.refresh_edit();
            }
        }
    }

    /// The card's spinner and the ghost's grey, before a command that
    /// tries the candidate again.
    fn show_checking(&mut self) {
        {
            let state = &mut self.editing.state;
            state.checking = true;
            if let Some(ghost) = state.ghost.as_mut() {
                ghost.ghosts.set_verdict(GhostVerdict::Checking);
            }
        }
        self.refresh_edit();
    }

    /// Sends an edit command now.
    pub(super) fn send_edit(&mut self, command: EditCommand) {
        if command == EditCommand::Add {
            let name = self
                .session
                .edit()
                .and_then(Edit::chosen)
                .and_then(|chosen| chosen.item_name().map(|name| name.as_str().to_owned()));
            let name = name.or_else(|| self.change_selects_after());
            let state = &mut self.editing.state;
            state.select_after = name;
            self.editing
                .editor_open
                .set(self.session.editor() == Editor::Open);
        }
        // The events draw the card from the try that follows, so it no
        // longer shows Checking.
        let was_checking = std::mem::take(&mut self.editing.state.checking);
        self.dispatch(Command::Edit(command));
        if was_checking {
            // A command that was rejected tried nothing: the ghost takes
            // back the verdict it had.
            let verdict = self
                .session
                .edit()
                .and_then(Edit::chosen)
                .and_then(|chosen| ghost_verdict(chosen.progress()));
            if let (Some(verdict), Some(ghost)) = (verdict, self.editing.state.ghost.as_mut()) {
                ghost.ghosts.set_verdict(verdict);
            }
            self.refresh_edit();
        }
    }

    /// Redraws what an edit event names.
    pub(super) fn edit_event(&mut self, event: EditEvent) {
        self.grip_event(&event);
        match event {
            EditEvent::Started => {
                self.stop_options_check();
                self.editing.repeats.row.forget();
                let generation = {
                    let state = &mut self.editing.state;
                    state.generation = state.generation.wrapping_add(1);
                    state.checking = false;
                    state.hover = None;
                    state.first_space = None;
                    state.generation
                };
                self.show_hint("");
                self.clear_ghost();
                self.refresh_edit();
                self.check_menu_soon(generation, MENU_STEP);
            }
            EditEvent::MenuChanged => self.refresh_edit(),
            EditEvent::Chosen => {
                // Another candidate: "Repeat…" is asked for again.
                self.editing.repeats.row.forget();
                self.update_ghost();
                self.refresh_edit();
                self.start_options_check();
            }
            EditEvent::PlaceBound
            | EditEvent::SettingChanged
            | EditEvent::Renamed
            | EditEvent::FixApplied
            | EditEvent::RepeatChanged
            | EditEvent::VerdictChanged => {
                self.update_ghost();
                self.refresh_edit();
                self.start_options_check();
            }
            EditEvent::ChangeStarted
            | EditEvent::DependantsIncluded
            | EditEvent::DragMoved
            | EditEvent::ParamPicked => {
                self.stop_options_check();
                self.update_ghost();
                self.refresh_edit();
            }
            EditEvent::OptionsChanged => {
                {
                    let session = &self.session;
                    if let Some(chosen) = session.edit().and_then(Edit::chosen) {
                        // Only fails once the card has shut down, with the
                        // window.
                        let _ = self
                            .editing
                            .card
                            .sender()
                            .send(EditCardInput::UpdateOptions(Box::new(
                                chosen.candidate().clone(),
                            )));
                    }
                }
                // An open search shows each answer as it comes.
                if let Some(name) = self.editing.search.setting() {
                    self.give_option_search(&name);
                    self.editing.search.refresh();
                }
            }
            EditEvent::Committed => {
                self.stop_options_check();
                let name = {
                    let state = &mut self.editing.state;
                    state.generation = state.generation.wrapping_add(1);
                    state.select_after.take()
                };
                self.clear_ghost();
                self.refresh_edit();
                let selection = {
                    let session = &self.session;
                    name.zip(session.drawable())
                        .and_then(|(name, (design, _))| {
                            stave_app::link::selection_named(design, &name)
                        })
                };
                if selection.is_some() {
                    self.dispatch(Command::Select(selection));
                }
                self.show_card();
            }
            EditEvent::Cancelled => {
                self.stop_options_check();
                {
                    let state = &mut self.editing.state;
                    state.generation = state.generation.wrapping_add(1);
                    state.checking = false;
                }
                self.close_change_dialog();
                self.clear_ghost();
                self.refresh_edit();
                self.show_card();
            }
            EditEvent::Rejected(why) => self.edit_toast(&why.text()),
            // Drawn by `grip_event`.
            EditEvent::GripsChanged | EditEvent::GripStopsFound(_) => {}
        }
    }

    /// Draws the menu or the card from the session, places them, and
    /// hides the part's card while an edit is open.
    pub(super) fn refresh_edit(&mut self) {
        let checking = self.editing.state.checking;
        let drawn = {
            let session = &self.session;
            draw_edit(&self.editing.menu, &self.editing.card, session, checking)
        };
        let open = drawn.is_open();
        self.editing.card_offers = match drawn {
            Drawn::Card(offers) => Some(offers),
            Drawn::Nothing | Drawn::Menu => None,
        };
        let open = self.draw_change(checking) || open;
        self.update_repeat_row();
        self.update_pending();
        self.editing
            .hold
            .from_space
            .set_visible(self.session.edit().is_some_and(|edit| {
                edit.chosen().is_none() && matches!(edit.picked(), Picked::Region(_))
            }));
        if open {
            self.place_edit_widgets();
        }
        self.update_edit_panel();
        self.update_face_button();
        self.request_render();
    }

    /// Shows the card in progress in the Edit panel, or how to start. When
    /// a card appears, the panel opens (where it last was, if it was
    /// closed) and its tab comes to the front, unless it shares a frame
    /// with the 3D view (it would hide the model being pointed at); the
    /// card takes the keyboard as it is mapped. When the edit is added or
    /// cancelled the panel says how to start again, and if it shares a
    /// frame with Details, Details comes back to the front.
    pub(super) fn update_edit_panel(&mut self) {
        let shows = self.editing.panel.update();
        let was = std::mem::replace(&mut self.editing.panel_shown, shows);
        let workbench = &self.widgets.workbench;
        if shows && !was {
            workbench.bring_forward_beside_view(super::workbench::PanelId::Edit);
        } else if !shows
            && was
            && workbench.share_a_frame(
                super::workbench::PanelId::Edit,
                super::workbench::PanelId::Details,
            )
            && workbench.is_shown(super::workbench::PanelId::Edit)
        {
            workbench.present(super::workbench::PanelId::Details);
        }
    }

    /// The menu has drawn what it was last shown: placed again, now that
    /// its height is its new entries'.
    pub(in crate::ui) fn menu_drawn(&self) {
        if self.editing.menu.widget().is_visible() {
            self.place_edit_widgets();
        }
    }

    /// The card says which setting's short list is open now.
    pub(in crate::ui) fn card_options_open(&mut self, setting: Option<String>) {
        self.editing.card_open_options = setting;
    }

    /// The menu near where it was asked for, inside the view (the cards
    /// are in the Edit panel).
    fn place_edit_widgets(&self) {
        let (width, height) = (
            f64::from(self.widgets.design.width()),
            f64::from(self.widgets.design.height()),
        );
        let (x, y) = self.editing.state.point.unwrap_or((width / 3.0, 80.0));
        let menu = self.editing.menu.widget();
        let menu_width = 340.0;
        let left = (x + 24.0).min(width - menu_width - f64::from(EDGE));
        // Clear of the dock at the bottom: as high as it must be to show
        // whole. A widget is measured with its margins, so the width asked
        // about holds the margins it was last placed with (measured at
        // 340 alone, a menu placed 593 pixels in "needs at least 913", as
        // GTK warned on every run that opened a menu).
        let across = 340_i32
            .saturating_add(menu.margin_start())
            .saturating_add(menu.margin_end());
        let (_, natural, _, _) = menu.measure(gtk::Orientation::Vertical, across);
        let natural = natural
            .saturating_sub(menu.margin_top())
            .saturating_sub(menu.margin_bottom());
        let top = (y - 60.0).min(height - f64::from(natural) - 96.0);
        menu.set_margin_start(crate::convert::int_of(left.max(f64::from(EDGE))));
        menu.set_margin_top(crate::convert::int_of(top.max(f64::from(EDGE))));
    }

    /// Works the menu's applicability out a step at a time, drawing what
    /// each step finds, until every entry is checked, something is chosen
    /// or the edit ends.
    fn check_menu_soon(&mut self, generation: u64, budget: Duration) {
        let input = self.input();
        glib::timeout_add_local_once(Duration::from_millis(16), move || {
            input.emit(EditingInput::CheckMenuStep { generation, budget }.into());
        });
    }

    pub(in crate::ui) fn check_menu_step(&mut self, generation: u64, budget: Duration) {
        if self.editing.state.generation != generation {
            return;
        }
        let before = {
            let session = &self.session;
            match session.edit() {
                Some(edit) if edit.chosen().is_none() => unchecked(edit),
                Some(_) | None => 0,
            }
        };
        if before == 0 {
            return;
        }
        // The step stops when the flag is raised, keeping what it found.
        let stop = Arc::new(AtomicBool::new(false));
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                std::thread::sleep(budget);
                stop.store(true, Ordering::Relaxed);
            });
        }
        let events = self.session.check_edit_menu(&stop);
        let after = self.session.edit().map_or(0, unchecked);
        if events.is_empty() && after < before {
            self.refresh_edit();
        }
        self.handle(events);
        if after > 0 {
            let next = if after < before {
                MENU_STEP
            } else {
                budget.saturating_mul(2).min(MENU_STEP_LIMIT)
            };
            self.check_menu_soon(generation, next);
        }
    }

    // ---- the options' checks, off the main thread ----

    /// Stops the check of the chosen candidate's options, if one runs.
    fn stop_options_check(&mut self) {
        let running = self.editing.options.take();
        if let Some((cancel, _)) = running {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Checks the chosen candidate's options on a worker thread, handing
    /// each answer to the session as it comes. A check already running for
    /// the candidate as it now stands goes on; one for the candidate as it
    /// was is stopped (its answers would be dropped). Scripts check the
    /// options they open themselves, so a picture never depends on how
    /// far a worker got.
    fn start_options_check(&mut self) {
        if self.editing.immediate {
            return;
        }
        let Some(job) = self.session.options_job() else {
            self.stop_options_check();
            return;
        };
        let running = self.editing.options.as_ref().map(|(_, ticket)| *ticket);
        if running == Some(job.ticket()) {
            return;
        }
        self.stop_options_check();
        if job.tries().is_empty() {
            return;
        }
        // The setting being searched, else the one whose short list is
        // open, is checked first.
        let job = match self
            .editing
            .search
            .setting()
            .or_else(|| self.editing.card_open_options.clone())
        {
            Some(open) => job.prefer(&open),
            None => job,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.editing.options = Some((cancel.clone(), job.ticket()));
        let (sender, receiver) = async_channel::unbounded::<FoundFit>();
        gio::spawn_blocking(move || {
            let _ = job.run(&cancel, &mut |found| {
                let _ = sender.send_blocking(found);
            });
        });
        let input = self.input();
        glib::spawn_future_local(async move {
            while let Ok(found) = receiver.recv().await {
                if input
                    .send(EditingInput::OptionFitFound(Unprinted(Box::new(found))).into())
                    .is_err()
                {
                    return;
                }
            }
        });
    }

    /// Opens the search over every option of a setting, checking them
    /// first (an edit script's `options` and `search` steps), with `query`
    /// typed.
    pub(super) fn open_setting_options(
        &mut self,
        setting: &str,
        query: &str,
    ) -> Result<(), String> {
        let events = self
            .session
            .check_setting_options(Some(setting), &AtomicBool::new(false));
        self.handle(events);
        if self.setting_has_options(setting) {
            self.open_option_search(setting, query);
            Ok(())
        } else {
            Err(format!("the card has no options to search for '{setting}'"))
        }
    }

    /// Opens a setting's short list on the card, checking its options
    /// first (an edit script's `choices` step).
    pub(super) fn open_setting_choices(&mut self, setting: &str) -> Result<(), String> {
        let events = self
            .session
            .check_setting_options(Some(setting), &AtomicBool::new(false));
        self.handle(events);
        // The window drew the card, so it knows what the card offers; the
        // card opens the list and brings it into sight when the main loop
        // next runs, after the redraws the events sent it.
        let offered = self
            .editing
            .card_offers
            .as_ref()
            .is_some_and(|offers| offers.has_short_list(setting));
        if !offered {
            return Err(format!("the card has no short list for '{setting}'"));
        }
        self.editing.card_open_options = Some(setting.to_owned());
        let card = self.editing.card.sender();
        // Only fails once the card has shut down, with the window.
        let _ = card.send(EditCardInput::OpenOptions(setting.to_owned()));
        let _ = card.send(EditCardInput::ScrollToOpenOptions);
        Ok(())
    }

    /// `true` while the card shows the button that puts back the setting
    /// a refusal blames (an edit script's `put back` step).
    pub(super) fn card_offers_put_back(&self) -> bool {
        self.editing
            .card_offers
            .as_ref()
            .is_some_and(CardOffers::put_back)
    }

    /// `true` when the chosen candidate's setting has options to search.
    fn setting_has_options(&self, setting: &str) -> bool {
        self.session
            .edit()
            .and_then(Edit::chosen)
            .and_then(|chosen| {
                chosen
                    .candidate()
                    .settings
                    .iter()
                    .find(|each| each.name.as_str() == setting)
                    .map(|each| !each.options.is_empty())
            })
            .unwrap_or(false)
    }

    /// What is chosen in the search dialog is set as a value typed on the
    /// card would be; closed, the keyboard goes back to the card.
    fn connect_option_search(&self) {
        let search = &self.editing.search;
        let input = self.input();
        search.connect_chosen(move |setting, text| {
            input.emit(EditingInput::OptionChosen { setting, text }.into());
        });
        let input = self.input();
        search.connect_closed(move || input.emit(EditingInput::OptionSearchClosed.into()));
    }

    /// Gives the search dialog the setting it searches, as the session has
    /// it now: it lists what a query matches from that copy, at once, with
    /// no call back into the window.
    fn give_option_search(&self, name: &str) {
        let setting = self
            .session
            .edit()
            .and_then(Edit::chosen)
            .and_then(|chosen| {
                chosen
                    .candidate()
                    .settings
                    .iter()
                    .find(|setting| setting.name.as_str() == name)
                    .cloned()
            });
        self.editing.search.connect_search(move |query| {
            setting
                .as_ref()
                .map(|setting| stave_app::choices::search(setting, query))
                .unwrap_or_default()
        });
    }

    /// An option chosen in the search dialog, set as a value typed on the
    /// card would be.
    pub(in crate::ui) fn option_chosen(&mut self, setting: &str, text: String) {
        let Ok(param) = parse_name(setting) else {
            return;
        };
        self.card_action(CardAction::Choose { param, text });
    }

    /// The search dialog closed: the keyboard goes back to the card, so
    /// Enter and Escape work there again.
    pub(in crate::ui) fn option_search_closed(&self) {
        let card = self.editing.card.widget();
        if card.is_visible() {
            card.grab_focus();
        }
    }

    /// Opens the search over a setting's options, `query` typed; the
    /// options' check is started again with that setting first.
    pub(super) fn open_option_search(&mut self, setting: &str, query: &str) {
        self.give_option_search(setting);
        self.editing.search.present(setting, query);
        // Checked with that setting first.
        self.stop_options_check();
        self.start_options_check();
    }

    // ---- the ghost ----

    /// Builds the chosen candidate's ghost from its last try: what it adds
    /// or changes, over the design as it is.
    fn update_ghost(&mut self) {
        let ghosts = {
            let session = &self.session;
            let chosen = session.edit().and_then(Edit::chosen);
            // A change of the design (a grip's drag) has a ghost too.
            let (verdict, candidate) = match (chosen, session.change()) {
                (Some(chosen), _) => (ghost_verdict(chosen.progress()), chosen.ghost()),
                (None, Some(change)) => (ghost_verdict(change.progress()), change.ghost()),
                (None, None) => (None, None),
            };
            let candidate = candidate.and_then(|opened| opened.design.as_ref());
            match (verdict, candidate, session.drawable()) {
                (Some(verdict), Some(candidate), Some((base, graph))) => Some(Ghosts::new(
                    base,
                    graph,
                    candidate,
                    BuildOptions {
                        extent: Extent::Cut,
                    },
                    verdict,
                )),
                _ => None,
            }
        };
        match ghosts {
            Some(ghosts) => {
                let posed = ghosts.graph().rest();
                self.editing.state.ghost = Some(GhostView {
                    ghosts,
                    posed,
                    uploaded: false,
                });
                self.request_render();
            }
            None => self.clear_ghost(),
        }
    }

    /// Takes the ghost away; the next frame puts the design's own scene
    /// back in the renderer.
    fn clear_ghost(&mut self) {
        if self.editing.state.ghost.take().is_some() {
            self.request_render();
        }
    }

    /// The region an edit is for, by name, when it began in a space.
    fn edit_region(&self, session: &Session, graph: &SceneGraph) -> Option<String> {
        match session.edit()?.picked() {
            Picked::Region(region) => graph.region(*region).map(|region| region.name.clone()),
            Picked::Face(_) | Picked::Between(_) | Picked::Group(_) => None,
        }
    }

    /// The spaces to light and the nodes to see through in a frame of the
    /// design: the space under the pointer, or the one the edit is for,
    /// and what covers it from the camera.
    pub(super) fn edit_marks(
        &self,
        session: &Session,
        graph: &SceneGraph,
        posed: &Posed,
        orbit: &Orbit,
    ) -> EditMarks {
        let state = &self.editing.state;
        let first = || {
            let name = state.first_space.as_deref()?;
            graph
                .regions()
                .iter()
                .find(|region| region.name == name)
                .map(|region| region.id)
        };
        let region = state
            .hover
            .or_else(|| match session.edit()?.picked() {
                Picked::Region(region) => Some(*region),
                Picked::Face(_) | Picked::Between(_) | Picked::Group(_) => None,
            })
            .or_else(|| session.edit().is_none().then(first).flatten());
        let Some(region) = region else {
            // A design of spaces with no parts yet (a new project's
            // envelope): its spaces show, waiting for the first click.
            return EditMarks {
                lit: pointing::spaces_waiting(graph, posed, session.edit().is_some()),
                covering: Vec::new(),
            };
        };
        EditMarks {
            lit: graph.region_volume(posed, region).into_iter().collect(),
            covering: graph.covering(posed, orbit, Subject::Region(region)),
        }
    }

    /// Draws a frame with the chosen candidate's ghost, when it has one:
    /// the candidate's scene, its ghosts tinted by the verdict, and what
    /// covers the space it goes in see-through. `None` draws the design
    /// as usual (and puts its scene back in the renderer first if a ghost
    /// had it).
    pub(super) fn render_ghost(&mut self, backdrop: Backdrop) -> Option<anyhow::Result<Ticket>> {
        if let Some(image) = self.render_removal(backdrop) {
            return Some(image);
        }
        let session = &self.session;
        let shows_ghost = self.editing.state.ghost.is_some() && self.state.focus.is_none();
        if !shows_ghost {
            let state = &mut self.state;
            if state.uploaded.is_none()
                && let Some(shown) = session.shown()
                && let Some((_, graph)) = shown.drawable()
                && let Ok(renderer) = &mut state.renderer
            {
                renderer.upload(graph);
                state.uploaded = Some(shown.clone());
            }
            return None;
        }
        let region = session
            .drawable()
            .and_then(|(_, graph)| self.edit_region(session, graph));
        let state = &mut self.state;
        let ghost = self.editing.state.ghost.as_mut()?;
        let orbit = state.orbit?;
        let (width, height) = state.size;
        let super::State {
            renderer,
            uploaded,
            render_queued,
            ..
        } = &mut *state;
        let Ok(renderer) = renderer else {
            return None;
        };
        *render_queued = false;
        // The ghost's scene replaces the design's in the renderer until the
        // ghost goes (a new result put the design's back: upload again).
        if uploaded.is_some() || !ghost.uploaded {
            renderer.upload(ghost.ghosts.graph());
            *uploaded = None;
            ghost.uploaded = true;
        }
        let graph = ghost.ghosts.graph();
        let count = graph.node_ids().count();
        let mut looks = vec![Look::Normal; count];
        let mut tints = vec![[0.0; 4]; count];
        if let Some(name) = &region {
            for node in covering_region(graph, &ghost.posed, &orbit, name) {
                if let Some(look) = looks.get_mut(node.index()) {
                    *look = Look::XRay;
                }
            }
        }
        let tint = ghost_tint(ghost.ghosts.verdict());
        for item in ghost.ghosts.ghosts() {
            if let Some(look) = looks.get_mut(item.node.index()) {
                *look = Look::XRay;
            }
            if let Some(slot) = tints.get_mut(item.node.index()) {
                *slot = tint;
            }
        }
        Some(renderer.submit(&Frame {
            scene: graph,
            posed: &ghost.posed,
            orbit,
            selection: &[],
            highlight: None,
            section: None,
            looks: Some(&looks),
            overlay: None,
            tint_cut_faces: true,
            lit: None,
            tints: Some(&tints),
            backdrop,
            width,
            height,
        }))
    }

    /// Places the repeat handles after a frame is drawn. The ghost's
    /// verdict is not said on the model: its tint says it (red when it
    /// doesn't fit), and the Edit panel says it in words.
    pub(super) fn place_ghost_verdict(&mut self) {
        self.place_repeat_handles();
    }

    // ---- driving it without a pointer (screenshots) ----

    /// Lights a space as if the pointer rested in it.
    pub(super) fn hover_region(&mut self, region: RegionId) {
        self.editing.state.hover = Some(region);
        self.request_render();
    }

    /// Selects a part as a click on one of its faces does.
    pub(super) fn click_face(&mut self, face: PartFace) {
        self.editing.state.face = Some(face);
        self.cancel_edit();
        self.dispatch(Command::Select(Some(Selection::Part(face.part))));
    }

    /// Works out every entry of the menu now, as the steps would.
    pub(super) fn check_menu_now(&mut self) {
        let never = AtomicBool::new(false);
        let events = self.session.check_edit_menu(&never);
        self.handle(events);
        self.refresh_edit();
    }

    /// Where a node set shows, as a point to open a menu at.
    pub(super) fn point_of(&self, volume: Option<Aabb>) -> (f64, f64) {
        let state = &self.state;
        volume
            .and_then(|volume| self.project(state, volume.centre()))
            .map_or((300.0, 300.0), |(x, y)| (f64::from(x), f64::from(y)))
    }
}

mod change_steps;
pub(in crate::ui) mod changes;
#[cfg(test)]
mod changes_tests;
mod editor_link;
mod grip_layer;
pub(in crate::ui) mod grip_panel;
mod grip_script;
mod grip_text;
mod grips;
pub(in crate::ui) mod hold;
pub(in crate::ui) mod item_tools;
mod links;
pub(super) use links::Hovered;
mod pointing;
mod repeat_handles;
pub(super) use repeat_handles::natural_size;
#[cfg(test)]
mod tests;
mod verbs;

pub(super) use pointing::{drawn_bounds, empty_state};
