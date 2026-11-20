//! The 3D view's floating controls that aren't buttons on the camera
//! column: the scene menu at the top left, the selection's motion slider
//! at the bottom right, and one chip per moving assembly ("Move: door_l,
//! door_r, drawer[top]") at the bottom left, which selects it so its
//! slider shows. Chips of assemblies with problems are ringed in the
//! problem's colour and carry the Parts tree's mark (a round dot for an
//! error, a square one for a warning), and say the problem to a screen
//! reader. They step aside while a problem is posed or a joint
//! is in focus.
//!
//! The chips and the scene menu are a relm4 component, [`DockComponent`]:
//! whether the controls are offered, whether there are chips, which way
//! they lie and the scenes to choose from are its model, only its update
//! changes that, and a chip or a scene's row only sends a [`DockInput`]
//! message. [`Dock`] owns its controller and keeps the setters as plain
//! methods; a chip or a scene chosen goes to the owner as the message it
//! chose when it made the dock. The scene menu is the dock's own: it
//! closes its menu on a choice before the owner hears it.

use adw::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::i18n::{fill_in, tr};
use stave_app::problems::Mark;
use stave_model::solved::AssemblyId;

use super::icons::{self, PathIcon};

/// One moving assembly's chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveChip {
    /// The assembly it selects.
    pub assembly: AssemblyId,
    /// Its name, the chip's words.
    pub name: String,
    /// Its problem mark, if it has a problem.
    pub mark: Option<Mark>,
}

/// The scenes to choose from and the one showing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenesShown {
    /// Each scene's title.
    pub titles: Vec<String>,
    /// The one showing, by its place in `titles`.
    pub current: usize,
}

/// The chips' and the scene menu's state.
#[derive(Debug)]
pub struct DockComponent {
    /// There are chips to show.
    has_moves: bool,
    /// The controls are offered (not while a problem is posed or a joint
    /// is in focus).
    offered: bool,
    /// The chips lie in a row (docked in a corner) or stand in a column
    /// (docked on the right edge).
    orientation: gtk::Orientation,
    /// The chips' natural line length while they lie in a row.
    natural_line_length: i32,
    /// The scenes to choose from; `None` keeps the menu as it was made
    /// ("Rest").
    scenes: Option<ScenesShown>,
}

/// The scene menu, made by [`Dock`] and handed to the component, which
/// owns its state.
#[derive(Clone, Debug)]
pub struct SceneMenu {
    /// Its floating box, shown while the controls are offered.
    scene_box: gtk::Box,
    /// The menu's button.
    button: gtk::MenuButton,
    /// The scene showing, on the button.
    label: gtk::Label,
    /// The scenes, in the button's popover.
    scenes: gtk::ListBox,
}

impl SceneMenu {
    /// The scene menu in its floating box, showing "Rest".
    fn new() -> SceneMenu {
        let label = gtk::Label::new(Some(&tr("Rest")));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = PathIcon::new(icons::SCENE, 15);
        icon.add_css_class("dim");
        let chevron = PathIcon::new(icons::CHEVRON_DOWN, 10);
        chevron.add_css_class("dim");
        content.append(&icon);
        content.append(&label);
        content.append(&chevron);
        let scenes = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["navigation-sidebar"])
            .build();
        let popover = gtk::Popover::builder().child(&scenes).build();
        let button = gtk::MenuButton::builder()
            .child(&content)
            .popover(&popover)
            .tooltip_text(tr("Scene"))
            .css_classes(["stave-scene"])
            .valign(gtk::Align::Center)
            .build();
        let scene_box = gtk::Box::builder()
            .css_classes(["stave-toolbar", "stave-scene-float"])
            .name("scene-menu")
            .build();
        scene_box.append(&button);
        SceneMenu {
            scene_box,
            button,
            label,
            scenes,
        }
    }
}

/// Fills the scene menu: a row per title, the current one in bold.
fn fill_scenes(scenes: &gtk::ListBox, shown: &ScenesShown) {
    while let Some(row) = scenes.row_at_index(0) {
        scenes.remove(&row);
    }
    for (index, title) in shown.titles.iter().enumerate() {
        let label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .margin_start(6)
            .margin_end(6)
            .build();
        let row = gtk::ListBoxRow::builder().child(&label).build();
        if index == shown.current {
            label.add_css_class("heading");
        }
        scenes.append(&row);
    }
}

/// What the chips are told.
#[derive(Debug)]
pub enum DockInput {
    /// Offer the controls, or take them away while a problem is posed or
    /// a joint is in focus.
    Offer(bool),
    /// Lay the chips out in a row (horizontal) or a column (vertical).
    SetMovesOrientation(gtk::Orientation),
    /// The moving assemblies' chips, the one whose slider shows marked
    /// current.
    ShowMoves {
        /// Each chip, in order.
        chips: Vec<MoveChip>,
        /// The assembly whose slider shows.
        current: Option<AssemblyId>,
    },
    /// People chose this assembly's chip.
    Chose(AssemblyId),
    /// Fill the scene menu.
    ShowScenes(ScenesShown),
    /// People chose the scene at this place in the scene menu.
    SceneChosen(usize),
}

/// What the chips and the scene menu tell their owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockOutput {
    /// People chose this assembly's chip.
    Move(AssemblyId),
    /// People chose the scene at this place in the scene menu.
    Scene(usize),
}

/// A chip: the tree's mark and the assembly's name, ringed for its
/// problem and saying it to a screen reader, marked current.
fn chip(
    chip: &MoveChip,
    current: Option<AssemblyId>,
    input: &relm4::Sender<DockInput>,
) -> gtk::Button {
    // The tree's mark beside the name: a round dot for an error, a
    // square one for a warning, so the chip's ring is not the only sign.
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&super::style::mark_dot(chip.mark));
    content.append(&gtk::Label::new(Some(&chip.name)));
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text(fill_in(
            &tr("Open or close {name}"),
            &[("name", &chip.name)],
        ))
        .css_classes(["stave-move"])
        .build();
    let problem = match chip.mark {
        Some(Mark::Error) => Some(("error", tr("Has an error"))),
        Some(Mark::Involved) => Some(("error", tr("Is part of an error"))),
        Some(Mark::Warning) => Some(("warning", tr("Has a warning"))),
        None => None,
    };
    if let Some((class, words)) = problem {
        button.add_css_class(class);
        button.update_property(&[gtk::accessible::Property::Description(&words)]);
    }
    if current == Some(chip.assembly) {
        button.add_css_class("current");
    }
    let input = input.clone();
    let assembly = chip.assembly;
    button.connect_clicked(move |_| input.emit(DockInput::Chose(assembly)));
    button
}

impl DockComponent {
    /// The scene showing, on the scene menu's button ("Rest" until the
    /// scenes are shown).
    fn scene_title(&self) -> String {
        self.scenes
            .as_ref()
            .and_then(|scenes| scenes.titles.get(scenes.current))
            .map_or_else(|| tr("Rest"), Clone::clone)
    }
}

#[relm4::component(pub)]
impl Component for DockComponent {
    type Init = SceneMenu;
    type Input = DockInput;
    type Output = DockOutput;
    type CommandOutput = ();

    view! {
        #[root]
        gtk::Box {
            set_spacing: 6,
            add_css_class: "stave-dock",
            add_css_class: "stave-moves",
            set_widget_name: "moves",
            #[watch]
            set_orientation: model.orientation,
            #[watch]
            set_visible: model.offered && model.has_moves,

            gtk::Label {
                set_label: &tr("Move:"),
                set_valign: gtk::Align::Start,
                add_css_class: "stave-dock-label",
            },
            #[name = "chips"]
            adw::WrapBox {
                set_child_spacing: 6,
                set_line_spacing: 6,
            },
        },

        #[local_ref]
        scene_box -> gtk::Box {
            #[watch]
            set_visible: model.offered,
        },

        #[local_ref]
        scene_label -> gtk::Label {
            #[watch]
            set_label: &model.scene_title(),
        },

        // Nothing to choose, nothing to open.
        #[local_ref]
        scene_button -> gtk::MenuButton {
            #[watch]
            set_sensitive: model.scenes.as_ref().is_none_or(|scenes| !scenes.titles.is_empty()),
        },

        #[local_ref]
        scene_list -> gtk::ListBox {
            connect_row_activated[sender] => move |_, row| {
                if let Ok(index) = usize::try_from(row.index()) {
                    sender.input_sender().emit(DockInput::SceneChosen(index));
                }
            },
        },
    }

    fn init(
        init: SceneMenu,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let SceneMenu {
            scene_box,
            button,
            label,
            scenes,
        } = init;
        let (scene_box, scene_button, scene_label, scene_list) =
            (&scene_box, &button, &label, &scenes);
        let mut model = DockComponent {
            has_moves: false,
            offered: true,
            orientation: gtk::Orientation::Horizontal,
            natural_line_length: 0,
            scenes: None,
        };
        let widgets = view_output!();
        model.natural_line_length = widgets.chips.natural_line_length();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: DockInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            DockInput::Offer(offered) => self.offered = offered,
            DockInput::SetMovesOrientation(orientation) => {
                self.orientation = orientation;
                // A column is one chip per line.
                widgets.chips.set_natural_line_length(match orientation {
                    gtk::Orientation::Vertical => 0,
                    _ => self.natural_line_length,
                });
            }
            DockInput::ShowMoves { chips, current } => {
                let wrap = &widgets.chips;
                while let Some(child) = wrap.first_child() {
                    wrap.remove(&child);
                }
                self.has_moves = !chips.is_empty();
                for each in &chips {
                    wrap.append(&chip(each, current, sender.input_sender()));
                }
            }
            DockInput::Chose(assembly) => {
                // Nobody listening is not a mistake.
                let _ = sender.output(DockOutput::Move(assembly));
            }
            DockInput::ShowScenes(scenes) => {
                fill_scenes(&widgets.scene_list, &scenes);
                self.scenes = Some(scenes);
            }
            DockInput::SceneChosen(index) => {
                // The menu closes on a choice, before the owner hears it.
                widgets.scene_button.popdown();
                let _ = sender.output(DockOutput::Scene(index));
            }
        }
        self.update_view(widgets, sender);
    }
}

/// The floating controls: it owns the [`DockComponent`] (dropping it
/// shuts the component down), passes its setters on as [`DockInput`]
/// messages, and its [`DockOutput`] goes to the owner's input. It has two
/// roots for the owner to place, the scene menu's box and the chips' box;
/// everything inside them is the dock's own.
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned.
pub struct Dock {
    /// The scene menu, in its floating box (top left).
    pub scene: gtk::Box,
    /// The moving assemblies' chips (bottom left).
    pub moves: gtk::Box,
    /// The scene menu's widgets, for tests.
    #[cfg(test)]
    scene_menu: SceneMenu,
    controller: Controller<DockComponent>,
}

impl Dock {
    /// The scene menu and the chips; the motion bar floats on its own
    /// (bottom right). A chip or a scene chosen goes to `owner` as
    /// `message(output)`.
    pub fn new<M: 'static>(
        owner: &relm4::Sender<M>,
        message: impl Fn(DockOutput) -> M + 'static,
    ) -> Dock {
        let scene_menu = SceneMenu::new();
        let scene = scene_menu.scene_box.clone();
        #[cfg(test)]
        let kept = scene_menu.clone();
        let controller = DockComponent::builder()
            .launch(scene_menu)
            .forward(owner, message);
        Dock {
            scene,
            moves: controller.widget().clone(),
            #[cfg(test)]
            scene_menu: kept,
            controller,
        }
    }

    /// The scenes to choose from (titles) and the one showing.
    pub fn show_scenes(&self, titles: &[String], current: usize) {
        self.send(DockInput::ShowScenes(ScenesShown {
            titles: titles.to_vec(),
            current,
        }));
    }

    /// Queues `input`; once the dock has shut down there is nobody to
    /// tell.
    fn send(&self, input: DockInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Lays the chips out in a row (docked in a corner) or a column
    /// (docked on the right edge), one chip per line.
    pub fn set_moves_orientation(&self, orientation: gtk::Orientation) {
        self.send(DockInput::SetMovesOrientation(orientation));
    }

    /// Offers the controls, or takes them away while a problem is posed
    /// or a joint is in focus.
    pub fn offer(&self, offered: bool) {
        self.send(DockInput::Offer(offered));
    }

    /// The moving assemblies' chips: each one's name and problem mark, the
    /// one whose slider shows marked current.
    pub fn show_moves(
        &self,
        moving: &[(AssemblyId, String, Option<Mark>)],
        current: Option<AssemblyId>,
    ) {
        let chips = moving
            .iter()
            .map(|(assembly, name, mark)| MoveChip {
                assembly: *assembly,
                name: name.clone(),
                mark: *mark,
            })
            .collect();
        self.send(DockInput::ShowMoves { chips, current });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{handle_each, on_gtk_thread, recorder, settle};

    /// A dock whose outputs go to `owner`.
    fn dock_for(owner: &relm4::Sender<DockOutput>) -> Dock {
        Dock::new(owner, std::convert::identity)
    }

    /// A dock whose outputs nobody hears.
    fn dock() -> Dock {
        let (owner, _) = recorder();
        dock_for(&owner)
    }

    /// The chips' buttons, in order.
    fn chips(dock: &Dock) -> Vec<gtk::Button> {
        let Some(wrap) = dock
            .moves
            .last_child()
            .and_then(|child| child.downcast::<adw::WrapBox>().ok())
        else {
            return Vec::new();
        };
        std::iter::successors(wrap.first_child(), gtk::Widget::next_sibling)
            .filter_map(|child| child.downcast::<gtk::Button>().ok())
            .collect()
    }

    fn moving() -> Vec<(AssemblyId, String, Option<Mark>)> {
        vec![
            (AssemblyId::from_usize(0), "door_l".to_owned(), None),
            (
                AssemblyId::from_usize(1),
                "door_r".to_owned(),
                Some(Mark::Error),
            ),
        ]
    }

    #[test]
    fn a_dropped_dock_is_freed() {
        on_gtk_thread(dock_is_freed);
    }

    /// Dropping the dock shuts its component down when the main loop next
    /// runs: the chips' box, the model and the message maker are all
    /// freed.
    fn dock_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let (owner, _) = recorder::<DockOutput>();
        let dock = Dock::new(&owner, move |output| {
            let _ = &held;
            output
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        dock.show_moves(&moving(), None);
        settle();
        let moves = dock.moves.downgrade();
        let scene = dock.scene.downgrade();
        let scenes = dock.scene_menu.scenes.downgrade();
        let input = dock.controller.sender().clone();
        assert!(moves.upgrade().is_some(), "the chips live while held");
        drop(dock);
        settle();
        assert!(moves.upgrade().is_none(), "the chips' box is freed");
        assert!(scene.upgrade().is_none(), "the scene menu's box is freed");
        assert!(scenes.upgrade().is_none(), "the scene menu's list is freed");
        assert!(
            input.send(DockInput::Offer(true)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the message maker is freed");
    }

    #[test]
    fn setters_change_the_chips_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    fn setters_do_not_call_back() {
        let (owner, heard) = recorder();
        let dock = dock_for(&owner);
        dock.show_moves(&moving(), Some(AssemblyId::from_usize(1)));
        settle();
        let shown = chips(&dock);
        assert_eq!(shown.len(), 2);
        assert!(shown[1].has_css_class("error"));
        assert!(shown[1].has_css_class("current"));
        assert!(dock.moves.is_visible() && dock.scene.is_visible());
        dock.offer(false);
        dock.set_moves_orientation(gtk::Orientation::Vertical);
        settle();
        assert!(!dock.moves.is_visible() && !dock.scene.is_visible());
        assert_eq!(dock.moves.orientation(), gtk::Orientation::Vertical);
        dock.offer(true);
        dock.show_moves(&[], None);
        settle();
        assert!(!dock.moves.is_visible(), "no chips, no box");
        assert!(dock.scene.is_visible());
        assert!(heard.borrow().is_empty(), "setters never call back");
    }

    #[test]
    fn a_click_handled_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(clicks_are_queued);
    }

    /// The old dock called its callback from inside the chip's own
    /// `clicked`, and the window answered by rebuilding the chips: the
    /// chip being clicked was taken away while GTK was still in its
    /// handler. Now a click only queues a message; the owner handles it
    /// after the update has returned, and the chips it asks for, and a
    /// click fired while it handles it, are queued too.
    fn clicks_are_queued() {
        let (owner, outputs) = relm4::channel();
        let dock = Rc::new(dock_for(&owner));
        let heard: Rc<RefCell<Vec<(AssemblyId, usize)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&dock);
        handle_each(outputs, move |output| {
            let DockOutput::Move(assembly) = output else {
                return;
            };
            let Some(dock) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            if first {
                // Rebuilding from here is queued: the chips stay.
                dock.show_moves(&moving()[..1], Some(assembly));
                if let Some(other) = chips(&dock).first() {
                    other.emit_clicked();
                }
            }
            log.borrow_mut().push((assembly, chips(&dock).len()));
        });
        dock.show_moves(&moving(), None);
        settle();
        let Some(door_r) = chips(&dock).pop() else {
            panic!("door_r has a chip");
        };
        door_r.emit_clicked();
        assert!(heard.borrow().is_empty(), "the click is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                (AssemblyId::from_usize(1), 2),
                (AssemblyId::from_usize(0), 1)
            ],
            "each output after its update, the inner click after the rebuild"
        );
        assert_eq!(chips(&dock).len(), 1);
    }

    #[test]
    fn the_scene_menu_shows_the_scenes_without_calling_back() {
        on_gtk_thread(scenes_shown);
    }

    /// Filling the scene menu is queued and never calls back: a row per
    /// scene, the current one's title on the button; no scenes leave the
    /// button insensitive.
    fn scenes_shown() {
        let (owner, heard) = recorder();
        let dock = dock_for(&owner);
        let menu = &dock.scene_menu;
        assert_eq!(menu.label.label(), "Rest");
        dock.show_scenes(&["Rest".to_owned(), "Open".to_owned()], 1);
        assert!(menu.scenes.row_at_index(0).is_none(), "only queued");
        settle();
        assert!(menu.scenes.row_at_index(1).is_some());
        assert!(menu.scenes.row_at_index(2).is_none());
        assert_eq!(menu.label.label(), "Open");
        assert!(menu.button.is_sensitive());
        dock.show_scenes(&[], 0);
        settle();
        assert!(menu.scenes.row_at_index(0).is_none());
        assert!(!menu.button.is_sensitive(), "nothing to choose");
        assert!(heard.borrow().is_empty(), "setters never call back");
    }

    #[test]
    fn choosing_a_scene_closes_the_menu_then_tells_the_owner() {
        on_gtk_thread(a_scene_chosen_closes_the_menu);
    }

    /// The dock closes its own scene menu when people choose a scene, and
    /// only then says which: the owner never reaches into it.
    fn a_scene_chosen_closes_the_menu() {
        let (owner, outputs) = relm4::channel();
        let dock = Rc::new(dock_for(&owner));
        let window = gtk::Window::new();
        window.set_child(Some(&dock.scene));
        window.present();
        dock.show_scenes(&["Rest".to_owned(), "Open".to_owned()], 0);
        settle();
        let button = dock.scene_menu.button.clone();
        button.popup();
        settle();
        let open =
            |button: &gtk::MenuButton| button.popover().is_some_and(|popover| popover.is_visible());
        assert!(open(&button), "the scene menu is open");
        let heard: Rc<RefCell<Vec<(DockOutput, bool)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let seen = button.clone();
        handle_each(outputs, move |output| {
            log.borrow_mut().push((output, open(&seen)));
        });
        let row = dock
            .scene_menu
            .scenes
            .row_at_index(1)
            .expect("a second scene");
        row.emit_activate();
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![(DockOutput::Scene(1), false)],
            "the owner hears the choice with the menu already closed"
        );
        assert!(!open(&button), "and stays closed");
        window.destroy();
        settle();
    }

    #[test]
    fn a_chip_shows_its_problem_by_shape_too() {
        on_gtk_thread(chips_show_marks);
    }

    /// Each chip with a problem carries the tree's mark (round for an
    /// error, square for a warning) and says the problem to a screen
    /// reader; a chip with none shows no mark.
    fn chips_show_marks() {
        let dock = dock();
        dock.show_moves(
            &[
                (
                    AssemblyId::from_usize(0),
                    "door_l".to_owned(),
                    Some(Mark::Error),
                ),
                (
                    AssemblyId::from_usize(1),
                    "door_r".to_owned(),
                    Some(Mark::Warning),
                ),
                (AssemblyId::from_usize(2), "drawer".to_owned(), None),
            ],
            None,
        );
        settle();
        let marks: Vec<_> = chips(&dock)
            .iter()
            .map(|chip| {
                let dot = chip
                    .first_child()
                    .and_then(|content| content.first_child())
                    .expect("the chip's mark");
                assert!(dot.has_css_class("stave-mark"));
                (
                    dot.is_visible(),
                    dot.has_css_class("error"),
                    dot.has_css_class("warning"),
                    gtk::test_accessible_has_property(chip, gtk::AccessibleProperty::Description),
                )
            })
            .collect();
        assert_eq!(
            marks,
            [
                (true, true, false, true),
                (true, false, true, true),
                (false, false, false, false),
            ]
        );
    }
}
