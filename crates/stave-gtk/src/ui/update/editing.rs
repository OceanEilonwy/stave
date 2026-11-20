//! Editing in the 3D view: the menu and the card, the option search, the
//! links in the Details and Edit panels, the grips, the item's changes,
//! holds, repeats and the finder's verbs. The handlers live in the
//! `edit_view` files; this group turns their widgets' outputs into
//! [`EditingInput`]s and hands each to its handler.

use crate::ui::edit_card::CardAction;
use crate::ui::edit_view::changes::ChangeAction;
use crate::ui::edit_view::grip_panel::PanelAction;
use crate::ui::edit_view::grip_panel::SwitchMenu;
use crate::ui::edit_view::hold::HoldCardOutput;
use crate::ui::edit_view::item_tools::{ItemToolsOutput, RepeatFoldOutput};
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
    /// The menu has drawn what it was shown: place it by its new size.
    MenuDrawn,
    /// The menu has placed itself here: floating labels keep clear of it.
    MenuPlaced(crate::ui::edit_menu::MenuPlace),
    /// The card asked for something.
    CardAction(CardAction),
    /// The setting whose short list is open on the card changed.
    CardOptionsOpen(Option<String>),
    /// The card can bring its open short list into sight only once the
    /// window is laid out (an edit script's `choices`): the window lays
    /// itself out and tells the card.
    CardWantsLayout,
    /// The `--edit-fixture` showcase's menu or card said something: it is
    /// only printed.
    Showcase(crate::ui::edit_demo::ShowcaseOutput),
    /// "Add onto" the face last clicked.
    EditFace,
    /// The pointer is over the 3D view at this point (none: it left).
    PointerAt(Option<(f64, f64)>),
    /// Sends an editing command now (after the card's spinner showed).
    SendEdit(EditCommand),
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
    OptionFitFound(Box<FoundFit>),
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
    /// The item's repeat controls said something.
    ItemRepeat(RepeatFoldOutput),
    /// The card's repeat row was unfolded.
    CardRepeatUnfolded,
    /// People changed the count or direction on the candidate's card.
    CardRepeatChanged {
        /// How many.
        count: usize,
        /// Along which axis.
        axis: Axis,
        /// How many counts the window had shown the stepper when people
        /// changed it (a later one replaces the change).
        shows: u64,
    },
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
    GripStopsFound(Box<FoundStops>),
    /// The full offer for a selected item was worked out on a worker
    /// thread.
    ItemOfferFound {
        /// The item.
        item: String,
        /// The shown design it was asked for, by address.
        design: usize,
        /// What the session offers for it.
        offer: Option<Box<stave_app::item_offer::ItemOffer>>,
    },
    /// A size's row was clicked: its editor opens.
    OpenSize(usize),
    /// Cancel or Escape in the size editor.
    CloseSize,
    /// The size editor's slider moved to a position (steps of
    /// `grip_values::SLIDER_STEPS`), or was let go there.
    SizeSlid {
        /// Where along it.
        position: i32,
        /// It was let go.
        released: bool,
    },
    /// A step button or an arrow key: whole units, up or down.
    SizeNudged(i32),
    /// "Back to …" in the size editor.
    SizeReset,
    /// Enter in the size editor's field, with what is typed.
    SizeTyped(String),
    /// Apply in the size editor.
    SizeApply,
    /// The wait before checking the size editor's value is over.
    CheckSize(u64),
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
        /// The menu that asked, closed once the range is read.
        menu: SwitchMenu,
    },
}

impl From<EditingInput> for WindowInput {
    fn from(message: EditingInput) -> WindowInput {
        WindowInput::Editing(message)
    }
}

/// Puts the editing widgets over the 3D view and wires them to
/// [`EditingInput`]s. The Details panel's links come as the inspector's
/// outputs (`update/selection.rs`).
pub(in crate::ui) fn wire(window: &mut Window) {
    window.connect_editing();
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
            EditingInput::MenuDrawn => self.menu_drawn(),
            EditingInput::MenuPlaced(place) => self.menu_placed(place),
            EditingInput::CardAction(action) => self.card_action(action),
            EditingInput::CardOptionsOpen(setting) => self.card_options_open(setting),
            EditingInput::CardWantsLayout => self.lay_out_for_card(),
            EditingInput::Showcase(output) => crate::ui::edit_demo::heard(&output),
            EditingInput::EditFace => self.edit_face(),
            EditingInput::PointerAt(point) => self.edit_hover(point),
            EditingInput::SendEdit(command) => self.send_edit(command),
            EditingInput::ChooseNow(declaration) => self.choose_now(declaration),
            EditingInput::CheckMenuStep { generation, budget } => {
                self.check_menu_step(generation, budget);
            }
            EditingInput::OptionFitFound(found) => {
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
            EditingInput::ItemRepeat(output) => self.item_repeat_heard(output),
            EditingInput::CardRepeatUnfolded => self.card_repeat_unfolded(),
            EditingInput::CardRepeatChanged { count, axis, shows } => {
                self.card_repeat_changed(count, axis, shows);
            }
            EditingInput::FinderActions(query) => {
                let actions = self.finder_actions(&query);
                self.widgets.finder.show_actions(actions);
            }
            EditingInput::RunFinderAction(index) => self.run_finder_action(index),
            EditingInput::GripPanel(action) => self.grip_panel_asked(action),
            EditingInput::ShowGrips(wanted) => self.show_grips(wanted),
            EditingInput::RelistGrips => self.relist_grips(),
            EditingInput::GripStopsFound(found) => {
                let events = self.session.grip_stops_found(*found);
                self.handle(events);
            }
            EditingInput::ItemOfferFound {
                item,
                design,
                offer,
            } => self.item_offer_found(&item, design, offer.map(|offer| *offer)),
            EditingInput::OpenSize(index) => self.open_size(index),
            EditingInput::CloseSize => self.close_size(),
            EditingInput::SizeSlid { position, released } => self.size_slid(position, released),
            EditingInput::SizeNudged(units) => self.size_nudged(units),
            EditingInput::SizeReset => self.size_reset(),
            EditingInput::SizeTyped(text) => self.size_typed(&text),
            EditingInput::SizeApply => self.apply_size(),
            EditingInput::CheckSize(ticket) => self.check_size(ticket),
            EditingInput::GripMakePlain { index, param } => self.grip_make_plain(index, param),
            EditingInput::GripMakeSolved {
                index,
                param,
                low,
                high,
                menu,
            } => self.grip_make_solved(index, param, (&low, &high), &menu),
        }
    }
}

#[cfg(test)]
pub(in crate::ui) mod tests {
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::ui::edit_view::unchecked;
    use crate::ui::gtk_test::{on_gtk_thread, settle, settle_until};
    use relm4::{ComponentController, Controller};
    use stave_app::edit::Picked;
    use stave_app::session::Selection;

    /// A fresh copy of the door fixture (a cabinet with a bay to add a
    /// door to) in a temporary folder of its own.
    fn door_project() -> PathBuf {
        door_project_named("editing-group")
    }

    /// A fresh copy of the door fixture in a temporary folder named for
    /// `tag`, so tests don't share one.
    pub(in crate::ui) fn door_project_named(tag: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("stave-gtk-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/door_hits_wall/cabinet.stave");
        std::fs::copy(source, folder.join("cabinet.stave")).unwrap();
        folder.canonicalize().unwrap()
    }

    /// Launches a project window on `folder`, as the application does.
    pub(in crate::ui) fn launch(
        application: &adw::Application,
        folder: PathBuf,
    ) -> Controller<Window> {
        let options = Options {
            layout: Some(std::env::temp_dir().join("stave-editing-test-no-layout")),
            ..Options::default()
        };
        let builder = Window::builder();
        let root = builder.widget().clone();
        let widgets = build_widgets(&root, application, &folder, &options).unwrap();
        builder
            .launch(WindowInit {
                widgets,
                folder,
                options,
            })
            .detach()
    }

    /// Queues an editing message for the window, as its widgets do.
    pub(in crate::ui) fn send(controller: &Controller<Window>, message: EditingInput) {
        controller.sender().send(message.into()).unwrap();
    }

    #[test]
    fn an_edit_runs_through_the_windows_queued_messages() {
        on_gtk_thread(edit_through_messages);
    }

    /// The editing group on a real window: each message (and each signal
    /// a widget fires while one is handled) is only queued, and is
    /// handled once the main loop runs: a click in the bay opens the
    /// menu, its entries are checked a step at a time, a pressed entry
    /// becomes the card after the short wait, the card's Cancel ends the
    /// edit, and a link's click selects what it names.
    fn edit_through_messages() {
        let application = adw::Application::new(
            Some("org.stave.WindowTest.Editing"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        if let Err(error) = application.register(gio::Cancellable::NONE) {
            panic!("the test's application registers: {error}");
        }
        let controller = launch(&application, door_project());
        let checked = settle_until(Duration::from_secs(60), || {
            controller.model().session.drawable().is_some()
        });
        assert!(checked, "the project is checked and drawable");
        let (bay, side_l) = {
            let model = controller.model();
            let (design, _) = model.session.drawable().unwrap();
            let bay = design
                .regions()
                .iter()
                .find(|r| r.name == "bay")
                .unwrap()
                .id;
            let side = design
                .parts()
                .iter()
                .find(|p| p.name == "side_l")
                .unwrap()
                .id;
            (bay, side)
        };

        // A click in the bay: queued, then the menu.
        send(
            &controller,
            EditingInput::Dispatch(Command::Edit(EditCommand::Start(Picked::Region(bay)))),
        );
        assert!(
            controller.model().session.edit().is_none(),
            "the click is only queued"
        );
        settle();
        assert!(controller.model().session.edit().is_some());
        assert!(controller.model().editing.menu.widget().is_visible());

        // The entries are checked a step at a time, from timers.
        let all_checked = settle_until(Duration::from_secs(60), || {
            controller
                .model()
                .session
                .edit()
                .is_some_and(|edit| unchecked(edit) == 0)
        });
        assert!(all_checked, "every entry of the menu is checked");

        // Pressing the Door entry: nothing is chosen under the press, the
        // card shows once the short wait is over.
        let button = {
            let model = controller.model();
            let door = model
                .session
                .edit()
                .unwrap()
                .menu()
                .iter()
                .find(|candidate| candidate.entry.label.as_str() == "Door")
                .unwrap()
                .declaration;
            let rows = model.editing.menu.model().rows().to_vec();
            rows.into_iter()
                .find(|(declaration, _)| *declaration == door)
                .map(|(_, button)| button)
                .unwrap()
        };
        // No borrow of the model is held while the button's signal runs.
        button.emit_clicked();
        assert!(
            controller
                .model()
                .session
                .edit()
                .is_some_and(|edit| edit.chosen().is_none()),
            "the press is only queued"
        );
        let chosen = settle_until(Duration::from_secs(30), || {
            controller
                .model()
                .session
                .edit()
                .is_some_and(|edit| edit.chosen().is_some())
        });
        assert!(chosen, "the Door entry is chosen");
        assert!(controller.model().editing.card.widget().is_visible());
        assert!(!controller.model().editing.menu.widget().is_visible());

        // Opening a short list on the card: the card tells the window,
        // which keeps which list is open (its options are checked first).
        let (param, toggle) = {
            let model = controller.model();
            let lists = model.editing.card.model().option_lists();
            let (param, toggle, _) = lists.into_iter().next().expect("the door has a short list");
            (param.as_str().to_owned(), toggle)
        };
        assert_eq!(controller.model().editing.card_open_options, None);
        toggle.set_active(true);
        assert_eq!(
            controller.model().editing.card_open_options,
            None,
            "only queued"
        );
        settle();
        assert_eq!(
            controller.model().editing.card_open_options.as_deref(),
            Some(param.as_str())
        );

        // The card's Cancel ends the edit.
        send(&controller, EditingInput::CardAction(CardAction::Cancel));
        let cancelled = settle_until(Duration::from_secs(30), || {
            controller.model().session.edit().is_none()
        });
        assert!(cancelled, "Cancel drops the edit");
        assert!(!controller.model().editing.card.widget().is_visible());

        // A link's click selects what it names, once handled.
        let part = Selection::Part(side_l);
        send(
            &controller,
            EditingInput::LinkClicked(LinkTarget::Select(part.clone())),
        );
        assert_ne!(controller.model().session.selection(), Some(&part));
        settle();
        assert_eq!(controller.model().session.selection(), Some(&part));

        controller.widget().close();
        settle();
        drop(controller);
        settle();
    }

    #[test]
    fn pins_keep_clear_of_where_the_menu_has_just_placed_itself() {
        on_gtk_thread(pins_clear_the_placed_menu);
    }

    /// The menu places itself and says where (`EditMenuOutput::Placed`):
    /// the panels the window keeps the problem pins clear of hold the
    /// menu's new place as soon as the window has heard it, never the
    /// margins it had before the queued `Place` was handled, and a pin
    /// wanted on that place is moved off it.
    fn pins_clear_the_placed_menu() {
        use crate::ui::edit_menu::EditMenuInput;
        use crate::ui::edit_view::{Rect, clear_of};
        let application = adw::Application::new(
            Some("org.stave.WindowTest.MenuPlace"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        if let Err(error) = application.register(gio::Cancellable::NONE) {
            panic!("the test's application registers: {error}");
        }
        let controller = launch(&application, door_project());
        // Shown, so the view has a size to keep the menu inside.
        controller.widget().set_default_size(1400, 1000);
        controller.widget().present();
        let checked = settle_until(Duration::from_secs(60), || {
            controller.model().session.drawable().is_some()
        });
        assert!(checked, "the project is checked and drawable");
        let bay = {
            let model = controller.model();
            let (design, _) = model.session.drawable().unwrap();
            design
                .regions()
                .iter()
                .find(|r| r.name == "bay")
                .unwrap()
                .id
        };
        send(
            &controller,
            EditingInput::Dispatch(Command::Edit(EditCommand::Start(Picked::Region(bay)))),
        );
        settle();
        assert!(controller.model().editing.menu.widget().is_visible());
        let before = controller.model().editing.menu_place;
        assert!(before.is_some(), "the menu said where it first went");

        // The menu is told to stand somewhere else: until it has handled
        // that and said so, the window keeps where it was.
        let menu = controller.model().editing.menu.sender().clone();
        // The window is laid out a few frames after it is presented,
        // later on a busy machine.
        let laid_out = settle_until(Duration::from_secs(10), || {
            let design = &controller.model().widgets.design;
            design.width() > 400 && design.height() > 300
        });
        assert!(laid_out, "the view is laid out");
        // The menu's entries are checked a step at a time, each step
        // drawing it and placing it again: wait for the last, so no later
        // placing of the window's own replaces the one asked for below.
        let all_checked = settle_until(Duration::from_secs(60), || {
            controller
                .model()
                .session
                .edit()
                .is_some_and(|edit| unchecked(edit) == 0)
        });
        assert!(all_checked, "every entry of the menu is checked");
        settle();
        let before = controller.model().editing.menu_place;
        let view = {
            let design = &controller.model().widgets.design;
            (f64::from(design.width()), f64::from(design.height()))
        };
        assert!(
            view.0 > 400.0 && view.1 > 300.0,
            "the view has a size: {view:?}"
        );
        // Where the window places it from, as `place_edit_widgets` does.
        menu.send(EditMenuInput::Place {
            point: (view.0 - 500.0, 40.0),
            view,
        })
        .unwrap();
        assert_eq!(controller.model().editing.menu_place, before, "only queued");
        settle();
        let place = controller
            .model()
            .editing
            .menu_place
            .expect("the menu said where it went");
        assert_ne!(Some(place), before, "it moved");
        let root = controller.model().editing.menu.widget().clone();
        assert_eq!(
            (place.start, place.top),
            (root.margin_start(), root.margin_top())
        );

        let panels = controller.model().floating_panels();
        let (view_width, view_height) = {
            let design = &controller.model().widgets.design;
            (design.width(), design.height())
        };
        let x = crate::convert::f32_of_int(place.start);
        let y = crate::convert::f32_of_int(place.top);
        let menu_rect = panels
            .iter()
            .find(|panel| panel.x == x && panel.y == y)
            .copied()
            .expect("the menu's new place is a panel to keep clear of");
        assert!(menu_rect.width > 0.0 && menu_rect.height > 0.0);
        // A pin wanted on the menu's new place is moved off it.
        let pin = Rect {
            x: x + 10.0,
            y: y + 10.0,
            width: 40.0,
            height: 24.0,
        };
        let room = (
            crate::convert::f32_of_int(view_width),
            crate::convert::f32_of_int(view_height),
        );
        let (dx, dy) = clear_of(pin, &panels, room).expect("a clear spot");
        let moved = Rect {
            x: pin.x + dx,
            y: pin.y + dy,
            ..pin
        };
        let overlaps = moved.x < menu_rect.x + menu_rect.width
            && menu_rect.x < moved.x + moved.width
            && moved.y < menu_rect.y + menu_rect.height
            && menu_rect.y < moved.y + moved.height;
        assert!(
            !overlaps,
            "the pin {moved:?} is clear of the menu {menu_rect:?}"
        );

        controller.widget().close();
        settle();
        drop(controller);
        settle();
    }

    /// Presses Delete on the window as GTK would, with no modifiers:
    /// whether the window took it.
    fn press_delete(controller: &Controller<Window>) -> bool {
        use gtk::glib::translate::IntoGlib;
        let keys = controller
            .widget()
            .observe_controllers()
            .into_iter()
            .filter_map(Result::ok)
            .filter_map(|found| found.downcast::<gtk::EventControllerKey>().ok())
            .find(|keys| {
                keys.name()
                    .is_some_and(|name| name == crate::ui::update::rendering::DELETE_KEYS)
            })
            .expect("the window has its Delete key controller");
        keys.emit_by_name::<bool>(
            "key-pressed",
            &[
                &gdk::Key::Delete.into_glib(),
                &0u32,
                &gdk::ModifierType::empty(),
            ],
        )
    }

    #[test]
    fn delete_is_passed_on_when_nothing_can_be_removed() {
        on_gtk_thread(delete_only_when_something_goes);
    }

    /// Delete outside a text field is decided at once: with nothing
    /// selected it is passed on to other widgets, and once a part is
    /// selected the window takes it and removes the part (saying first
    /// what goes).
    fn delete_only_when_something_goes() {
        let application = adw::Application::new(
            Some("org.stave.WindowTest.Delete"),
            gio::ApplicationFlags::NON_UNIQUE,
        );
        if let Err(error) = application.register(gio::Cancellable::NONE) {
            panic!("the test's application registers: {error}");
        }
        let controller = launch(&application, door_project());
        let checked = settle_until(Duration::from_secs(60), || {
            controller.model().session.drawable().is_some()
        });
        assert!(checked, "the project is checked and drawable");
        assert!(
            controller.model().session.selection().is_none(),
            "nothing is selected at first"
        );

        assert!(
            !press_delete(&controller),
            "with nothing to remove, Delete is passed on"
        );
        settle();
        assert!(
            controller.model().session.change().is_none(),
            "nothing is being removed"
        );

        let side_l = {
            let model = controller.model();
            let (design, _) = model.session.drawable().unwrap();
            design
                .parts()
                .iter()
                .find(|p| p.name == "side_l")
                .unwrap()
                .id
        };
        send(
            &controller,
            EditingInput::LinkClicked(LinkTarget::Select(Selection::Part(side_l))),
        );
        settle();
        assert!(
            press_delete(&controller),
            "with a part selected, the window takes Delete"
        );
        let removing = settle_until(Duration::from_secs(30), || {
            controller.model().session.change().is_some()
        });
        assert!(removing, "Delete starts removing the selected part");

        controller.widget().close();
        settle();
        drop(controller);
        settle();
    }
}
