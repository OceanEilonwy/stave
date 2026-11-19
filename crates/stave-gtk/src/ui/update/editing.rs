//! Editing in the 3D view: the menu and the card, the option search, the
//! links in the Details and Edit panels, the grips, the item's changes,
//! holds, repeats and the finder's verbs. The handlers live in the
//! `edit_view` files; this group turns their widgets' outputs into
//! [`EditingInput`]s and hands each to its handler.

use super::Unprinted;
use crate::ui::edit_card::CardAction;
use crate::ui::edit_view::changes::ChangeAction;
use crate::ui::edit_view::grip_panel::PanelAction;
use crate::ui::edit_view::hold::HoldCardOutput;
use crate::ui::edit_view::item_tools::ItemToolsOutput;
// The window's own items and imports, as the handlers had them in mod.rs.
use crate::ui::*;
use stave_app::edit::Declaration;
use stave_app::editing::EditCommand;
use stave_app::gripping::FoundStops;
use stave_app::grips::GripSelection;
use stave_app::inspect::LinkTarget;
use stave_app::option_fits::FoundFit;
use stave_lang::ast::Ident;
use stave_lang::placement::Axis;

/// What the editing group is told.
#[derive(Debug)]
pub(in crate::ui) enum EditingInput {
    /// Sends this command to the session.
    Dispatch(Command),
    /// Delete: removes the selection, saying first what goes.
    RemoveSelected,
    /// An entry of the editing menu was chosen.
    MenuChosen(Declaration),
    /// The card asked for something.
    CardAction(CardAction),
    /// "Add onto" the face last clicked.
    EditFace,
    /// The pointer is over the 3D view at this point (none: it left).
    PointerAt(Option<(f64, f64)>),
    /// Sends an editing command now (after the card's spinner showed).
    SendEdit(EditCommand),
    /// Sends an editing command once the signal that asked is done.
    SendEditLater(EditCommand),
    /// Chooses the menu entry for this declaration now.
    ChooseNow(Declaration),
    /// The next step of checking the menu's entries.
    CheckMenuStep {
        /// The edit the check is for.
        generation: u64,
        /// How long the check may go on.
        budget: Duration,
    },
    /// An option's fit was found on the worker thread.
    OptionFitFound(Unprinted<Box<FoundFit>>),
    /// An option was chosen in the search dialog.
    OptionChosen {
        /// The setting.
        setting: String,
        /// The option's text.
        text: String,
    },
    /// The search dialog closed.
    OptionSearchClosed,
    /// A link in the Details or Edit panel was clicked.
    LinkClicked(LinkTarget),
    /// The pointer is on a link (none: it left).
    LinkHovered(Option<LinkTarget>),
    /// The removal's dialog was answered: include the dependants or not.
    DependantsAnswered(bool),
    /// The change's card asked for something.
    ChangeAction(ChangeAction),
    /// "Hold a distance" from the space's menu.
    HoldFromSpace,
    /// The hold card asked for something.
    HoldCard(HoldCardOutput),
    /// The item's tools asked for something.
    ItemTools(ItemToolsOutput),
    /// The item's repeat row was unfolded.
    ItemRepeatUnfolded,
    /// The card's repeat row was unfolded.
    CardRepeatUnfolded,
    /// A repeat count or direction changed.
    RepeatChanged {
        /// How many.
        count: usize,
        /// Along which axis.
        axis: Axis,
    },
    /// The repeat count's handle was pressed here.
    CountDragBegun {
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// The repeat count's handle moved this far.
    CountDragged {
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// The repeat count's handle was let go.
    CountDragEnded,
    /// A boundary's handle was pressed here.
    BoundaryDragBegun {
        /// The handle.
        index: usize,
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// A boundary's handle moved this far.
    BoundaryDragged {
        /// The handle.
        index: usize,
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// A boundary's handle was let go.
    BoundaryDragEnded,
    /// The finder asks which verbs a query names.
    FinderActions(String),
    /// A verb was chosen in the finder.
    RunFinderAction(usize),
    /// The grips' panel asked for something.
    GripPanel(PanelAction),
    /// Lists the grips of this selection now.
    ShowGrips(Option<GripSelection>),
    /// Lists the grips shown again.
    RelistGrips,
    /// A grip's stops were found on the worker thread.
    GripStopsFound(Unprinted<Box<FoundStops>>),
    /// A grip's number was clicked: it turns into an entry.
    OpenGripEntry(usize),
    /// A value was typed into a grip's entry.
    GripTyped {
        /// The grip.
        index: usize,
        /// What was typed.
        text: String,
    },
    /// Escape in a grip's entry.
    CloseGripTyping(usize),
    /// A grip's range switch turned off.
    GripMakePlain {
        /// The grip.
        index: usize,
        /// Its size.
        param: Ident,
    },
    /// "Make it solved" on a grip's menu.
    GripMakeSolved {
        /// The grip.
        index: usize,
        /// Its size.
        param: Ident,
        /// The range's low end, as typed.
        low: String,
        /// The range's high end, as typed.
        high: String,
        /// The menu, closed once the range is read.
        menu: gtk::MenuButton,
    },
    /// A grip's handle was pressed.
    GripPressed(usize),
    /// A grip's handle moved this far.
    GripMoved {
        /// Across.
        dx: f64,
        /// Down.
        dy: f64,
        /// Ctrl is held: round the value.
        round: bool,
    },
    /// A grip's handle was let go.
    GripReleased,
    /// An arrow key moved a grip.
    GripStepped {
        /// The grip.
        index: usize,
        /// How many steps, and which way.
        steps: i32,
        /// Ctrl is held: round the value.
        round: bool,
    },
    /// Sends the dragged value waiting, if any.
    FlushDrag,
}

impl From<EditingInput> for WindowInput {
    fn from(message: EditingInput) -> WindowInput {
        WindowInput::Editing(message)
    }
}

/// Puts the editing widgets over the 3D view and wires them (and the
/// Details panel's links) to [`EditingInput`]s.
pub(in crate::ui) fn wire(window: &mut Window) {
    window.connect_editing();
    window.connect_links(&window.widgets.inspector.links);
}

impl Window {
    /// Handles an [`EditingInput`].
    pub(in crate::ui) fn update_editing(&mut self, message: EditingInput) {
        match message {
            EditingInput::Dispatch(command) => self.dispatch(command),
            EditingInput::RemoveSelected => {
                self.remove_selected();
            }
            EditingInput::MenuChosen(declaration) => self.choose_later(declaration),
            EditingInput::CardAction(action) => self.card_action(action),
            EditingInput::EditFace => self.edit_face(),
            EditingInput::PointerAt(point) => self.edit_hover(point),
            EditingInput::SendEdit(command) => self.send_edit(command),
            EditingInput::SendEditLater(command) => self.send_edit_later(command),
            EditingInput::ChooseNow(declaration) => self.choose_now(declaration),
            EditingInput::CheckMenuStep { generation, budget } => {
                self.check_menu_step(generation, budget);
            }
            EditingInput::OptionFitFound(Unprinted(found)) => {
                let events = self.session.option_fit_found(*found);
                self.handle(events);
            }
            EditingInput::OptionChosen { setting, text } => self.option_chosen(&setting, text),
            EditingInput::OptionSearchClosed => self.option_search_closed(),
            EditingInput::LinkClicked(target) => self.link_clicked(target),
            EditingInput::LinkHovered(target) => self.link_hovered(target),
            EditingInput::DependantsAnswered(include) => self.answer_dependants(include),
            EditingInput::ChangeAction(action) => self.send_edit_later(action.command()),
            EditingInput::HoldFromSpace => self.hold_from_space(),
            EditingInput::HoldCard(output) => self.hold_card_asked(output),
            EditingInput::ItemTools(output) => self.item_tools_asked(output),
            EditingInput::ItemRepeatUnfolded => self.update_item_tools(),
            EditingInput::CardRepeatUnfolded => {
                self.update_repeat_row();
                self.request_render();
            }
            EditingInput::RepeatChanged { count, axis } => {
                self.repeat_selected(count, Some(axis));
            }
            EditingInput::CountDragBegun { x, y } => self.count_drag_begin(x, y),
            EditingInput::CountDragged { x, y } => self.count_drag_update(x, y),
            EditingInput::CountDragEnded => self.count_drag_end(),
            EditingInput::BoundaryDragBegun { index, x, y } => {
                self.boundary_drag_begun(index, x, y);
            }
            EditingInput::BoundaryDragged { index, x, y } => self.boundary_dragged(index, x, y),
            EditingInput::BoundaryDragEnded => self.boundary_drag_ended(),
            EditingInput::FinderActions(query) => {
                let actions = self.finder_actions(&query);
                self.widgets.finder.show_actions(actions);
            }
            EditingInput::RunFinderAction(index) => self.run_finder_action(index),
            EditingInput::GripPanel(action) => self.grip_panel_asked(action),
            EditingInput::ShowGrips(wanted) => self.show_grips(wanted),
            EditingInput::RelistGrips => self.relist_grips(),
            EditingInput::GripStopsFound(Unprinted(found)) => {
                let events = self.session.grip_stops_found(*found);
                self.handle(events);
            }
            EditingInput::OpenGripEntry(index) => self.open_entry(index),
            EditingInput::GripTyped { index, text } => self.apply_typed(index, &text),
            EditingInput::CloseGripTyping(index) => self.close_typing(index),
            EditingInput::GripMakePlain { index, param } => self.grip_make_plain(index, param),
            EditingInput::GripMakeSolved {
                index,
                param,
                low,
                high,
                menu,
            } => self.grip_make_solved(index, param, (&low, &high), &menu),
            EditingInput::GripPressed(index) => self.grip_pressed(index),
            EditingInput::GripMoved { dx, dy, round } => self.grip_moved(dx, dy, round),
            EditingInput::GripReleased => self.grip_released(),
            EditingInput::GripStepped {
                index,
                steps,
                round,
            } => self.grip_stepped(index, steps, round),
            EditingInput::FlushDrag => self.flush_drag(),
        }
    }
}
