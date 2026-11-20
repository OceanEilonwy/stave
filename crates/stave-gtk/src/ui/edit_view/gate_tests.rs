//! Editing in 3D while the text doesn't read: every way in is refused the
//! same way, by the one gate (`Window::refuse_while_unreadable`, asked by
//! `Window::dispatch` for every edit command), and works again once the
//! text reads.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use relm4::{ComponentController, Controller};
use stave_app::edit::Picked;
use stave_app::editing::EditCommand;
use stave_app::inspect::LinkTarget;
use stave_app::session::{Command, Selection};
use stave_ide::{Analysis, Overlay};
use stave_model::ids::RegionId;
use stave_scene::Target;

use super::hold::HoldStep;
use super::item_tools::ItemToolsOutput;
use crate::ui::Window;
use crate::ui::gtk_test::{on_gtk_thread, settle, settle_until};
use crate::ui::update::editing::EditingInput;
use crate::ui::update::editing::tests::{door_project_named, launch, send};
use crate::ui::update::project::ProjectInput;
use crate::ui::update::rendering::RenderingInput;

/// The project's analysis as the editor would give it, with the cabinet's
/// file as `text` (`None`: as on disk).
fn analysed(folder: &Path, text: Option<&str>) -> Rc<Analysis> {
    let mut overlay = Overlay::new();
    if let Some(text) = text {
        overlay.insert(folder.join("cabinet.stave"), text);
    }
    Rc::new(Analysis::load(folder, &overlay).expect("the project loads"))
}

/// Tells the window the editor's analysis changed, as the Code panel does.
fn hear_analysis(controller: &Controller<Window>, analysis: Rc<Analysis>) {
    controller
        .sender()
        .send(ProjectInput::Analysed(analysis).into())
        .unwrap();
    settle();
}

/// Nothing is being edited or changed, no hold waits for its second
/// pick, and the line low in the view says why editing in 3D is off.
fn refused(controller: &Controller<Window>, what: &str) {
    settle();
    let model = controller.model();
    assert!(model.session.edit().is_none(), "{what}: no edit started");
    assert!(
        model.session.change().is_none(),
        "{what}: no change started"
    );
    assert!(
        matches!(model.editing.hold.step, HoldStep::Idle),
        "{what}: no hold started"
    );
    assert!(
        model.widgets.block_box.is_visible(),
        "{what}: the line says why"
    );
}

/// A click in the middle of the space `region` as the view shows it now
/// (worked out at the click: the view frames itself as it is laid out),
/// as the view's gesture sends it.
fn click_in(controller: &Controller<Window>, region: RegionId) {
    let at = {
        let model = controller.model();
        let (_, graph) = model.session.drawable().unwrap();
        let posed = model.state.posed.as_ref().unwrap();
        let at = model.point_of(graph.region_volume(posed, region));
        assert!(
            matches!(model.pointed_at(at.0, at.1).0, Some(Target::Region(_))),
            "the click lands in a space"
        );
        at
    };
    controller
        .sender()
        .send(RenderingInput::Picked { x: at.0, y: at.1 }.into())
        .unwrap();
    settle();
}

#[test]
fn every_way_into_editing_in_3d_waits_for_the_text_to_read() {
    on_gtk_thread(every_way_in_waits);
}

fn every_way_in_waits() {
    let application = adw::Application::new(
        Some("org.stave.WindowTest.Gate"),
        gtk::gio::ApplicationFlags::NON_UNIQUE,
    );
    if let Err(error) = application.register(gtk::gio::Cancellable::NONE) {
        panic!("the test's application registers: {error}");
    }
    let folder = door_project_named("editing-gate");
    let controller = launch(&application, folder.clone());
    controller.widget().set_default_size(1400, 1000);
    controller.widget().present();
    let checked = settle_until(Duration::from_secs(60), || {
        controller.model().session.drawable().is_some()
    });
    assert!(checked, "the project is checked and drawable");
    // Drawn, so the view has a size and a pose to point into.
    assert!(settle_until(Duration::from_secs(10), || {
        let model = controller.model();
        model.state.posed.is_some() && model.widgets.design.width() > 0
    }));
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

    // Readable: the side is selected (a link's click), and its item's
    // tools show.
    hear_analysis(&controller, analysed(&folder, None));
    send(
        &controller,
        EditingInput::LinkClicked(LinkTarget::Select(Selection::Part(side_l))),
    );
    settle();
    {
        let model = controller.model();
        assert_eq!(model.session.selection(), Some(&Selection::Part(side_l)));
        assert!(!model.widgets.block_box.is_visible());
    }
    assert!(
        settle_until(Duration::from_secs(10), || controller
            .model()
            .editing
            .tools
            .root
            .is_visible()),
        "the item's tools show while the text reads"
    );

    // The text stops reading.
    let text = std::fs::read_to_string(folder.join("cabinet.stave")).unwrap();
    let broken = text.replacen("length: 600mm", "length: = 600mm", 1);
    hear_analysis(&controller, analysed(&folder, Some(&broken)));
    {
        let model = controller.model();
        assert!(model.widgets.block_box.is_visible(), "the line says why");
        let line = &model.widgets.block_line;
        assert!(
            line.justify() == gtk::Justification::Center && line.hexpands(),
            "its lines are centred in its chip"
        );
        assert!(
            model.widgets.block_line.label().contains("cabinet.stave"),
            "it names the file: {}",
            model.widgets.block_line.label()
        );
        assert!(
            !model.editing.face_button.is_visible(),
            "no face button to press"
        );
        assert!(
            !model.editing.tools.root.is_visible(),
            "no item's tools to use"
        );
    }

    // Each way in does nothing but say why.
    click_in(&controller, bay);
    refused(&controller, "a click in the bay");
    assert_eq!(
        controller.model().session.selection(),
        Some(&Selection::Part(side_l)),
        "the selection is kept: the edit never began"
    );
    send(
        &controller,
        EditingInput::Dispatch(Command::Edit(EditCommand::Start(Picked::Region(bay)))),
    );
    refused(&controller, "an edit started by command");
    send(&controller, EditingInput::RemoveSelected);
    refused(&controller, "Delete");
    send(
        &controller,
        EditingInput::ItemTools(ItemToolsOutput::Rename("left_side".to_owned())),
    );
    refused(&controller, "the item's rename");
    send(
        &controller,
        EditingInput::ItemTools(ItemToolsOutput::Mirror),
    );
    refused(&controller, "the item's mirror");
    send(&controller, EditingInput::ItemTools(ItemToolsOutput::Hold));
    refused(&controller, "the item's hold");
    send(
        &controller,
        EditingInput::ItemTools(ItemToolsOutput::Remove),
    );
    refused(&controller, "the item's remove");
    send(&controller, EditingInput::HoldFromSpace);
    refused(&controller, "the space's hold");
    send(
        &controller,
        EditingInput::Dispatch(Command::Edit(EditCommand::BeginDrag(0))),
    );
    refused(&controller, "a grip's drag");

    // The text reads again: the same click opens the menu.
    hear_analysis(&controller, analysed(&folder, None));
    assert!(!controller.model().widgets.block_box.is_visible());
    click_in(&controller, bay);
    assert!(
        controller.model().session.edit().is_some(),
        "a click in the bay opens its menu again"
    );
    send(
        &controller,
        EditingInput::Dispatch(Command::Edit(EditCommand::Cancel)),
    );
    settle();
    send(&controller, EditingInput::RemoveSelected);
    settle();
    controller.widget().close();
    settle();
    drop(controller);
    settle();
}
