//! The 3D view's floating controls that aren't buttons on the camera
//! column: the scene menu at the top left, the selection's motion slider
//! at the bottom right, and one chip per moving assembly ("Move: door_l,
//! door_r, drawer[top]") at the bottom left, which selects it so its
//! slider shows. Chips of assemblies with problems are ringed in the
//! problem's colour. They step aside while a problem is posed or a joint
//! is in focus.
//!
//! The chips are a relm4 component, [`DockComponent`]: whether the
//! controls are offered, whether there are chips and which way they lie
//! are its model, only its update changes that, and a chip only sends a
//! [`DockInput`] message. [`Dock`] keeps the older API in front of it
//! until the window moves to relm4 too.

use std::rc::Rc;

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::problems::Mark;
use stave_model::solved::AssemblyId;

use super::chrome::MotionBar;
use super::relm4_shim::Bridge;

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

/// The chips' state.
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
}

/// What the chips are made with.
#[derive(Debug)]
pub struct DockInit {
    /// The scene menu's floating box, shown while the controls are
    /// offered.
    pub scene: gtk::Box,
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
}

/// What the chips tell their owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockOutput {
    /// People chose this assembly's chip.
    Move(AssemblyId),
}

/// A chip: the assembly's name, ringed for its problem, marked current.
fn chip(
    chip: &MoveChip,
    current: Option<AssemblyId>,
    input: &relm4::Sender<DockInput>,
) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(&chip.name)
        .tooltip_text(format!("Open or close {}", chip.name))
        .css_classes(["stave-move"])
        .build();
    match chip.mark {
        Some(Mark::Error | Mark::Involved) => button.add_css_class("error"),
        Some(Mark::Warning) => button.add_css_class("warning"),
        None => {}
    }
    if current == Some(chip.assembly) {
        button.add_css_class("current");
    }
    let input = input.clone();
    let assembly = chip.assembly;
    button.connect_clicked(move |_| input.emit(DockInput::Chose(assembly)));
    button
}

#[relm4::component(pub)]
impl Component for DockComponent {
    type Init = DockInit;
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
                set_label: "Move:",
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
        scene -> gtk::Box {
            #[watch]
            set_visible: model.offered,
        }
    }

    fn init(
        init: DockInit,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let DockInit { scene } = init;
        let mut model = DockComponent {
            has_moves: false,
            offered: true,
            orientation: gtk::Orientation::Horizontal,
            natural_line_length: 0,
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
        }
        self.update_view(widgets, sender);
    }
}

/// The floating controls: it owns the [`DockComponent`] (dropping it
/// shuts the component down), passes its setters on as [`DockInput`]
/// messages and its [`DockOutput`] to the callback connected with
/// [`Dock::connect_outputs`].
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned.
pub struct Dock {
    /// The scene menu, in its floating box (top left).
    pub scene: gtk::Box,
    /// The moving assemblies' chips (bottom left).
    pub moves: gtk::Box,
    bridge: Bridge<DockComponent>,
}

impl Dock {
    /// The controls round the scene menu; the motion bar floats on its
    /// own (bottom right).
    pub fn new(scene: &gtk::MenuButton, motion: &MotionBar) -> Rc<Dock> {
        let scene_box = gtk::Box::builder()
            .css_classes(["stave-toolbar", "stave-scene-float"])
            .name("scene-menu")
            .build();
        scene_box.append(scene);
        motion.root.set_margin_bottom(0);
        let bridge = Bridge::<DockComponent>::launch(DockInit {
            scene: scene_box.clone(),
        });
        Rc::new(Dock {
            scene: scene_box,
            moves: bridge.widget().clone(),
            bridge,
        })
    }

    /// Lays the chips out in a row (docked in a corner) or a column
    /// (docked on the right edge), one chip per line.
    pub fn set_moves_orientation(&self, orientation: gtk::Orientation) {
        self.bridge
            .send(DockInput::SetMovesOrientation(orientation));
    }

    /// Calls `callback` with each chip people choose from now on, instead
    /// of the one connected before.
    pub fn connect_outputs(&self, callback: impl Fn(DockOutput) + 'static) {
        self.bridge.connect(callback);
    }

    /// Offers the controls, or takes them away while a problem is posed
    /// or a joint is in focus.
    pub fn offer(&self, offered: bool) {
        self.bridge.send(DockInput::Offer(offered));
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
        self.bridge.send(DockInput::ShowMoves { chips, current });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// A dock round a bare menu button and a fresh motion bar.
    fn dock() -> Rc<Dock> {
        Dock::new(&gtk::MenuButton::new(), &MotionBar::new())
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
    /// runs: the chips' box, the model and the callback are all freed.
    fn dock_is_freed() {
        let dock = dock();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        dock.connect_outputs(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        dock.show_moves(&moving(), None);
        settle();
        let moves = dock.moves.downgrade();
        let scene = dock.scene.downgrade();
        let input = dock.bridge.sender().clone();
        assert!(moves.upgrade().is_some(), "the chips live while held");
        drop(dock);
        settle();
        assert!(moves.upgrade().is_none(), "the chips' box is freed");
        assert!(scene.upgrade().is_none(), "the scene menu's box is freed");
        assert!(
            input.send(DockInput::Offer(true)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn setters_change_the_chips_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    fn setters_do_not_call_back() {
        let dock = dock();
        let calls = Rc::new(RefCell::new(0));
        let counted = Rc::clone(&calls);
        dock.connect_outputs(move |_| *counted.borrow_mut() += 1);
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
        assert_eq!(*calls.borrow(), 0, "setters never call back");
    }

    #[test]
    fn a_click_handled_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(clicks_are_queued);
    }

    /// The old dock called its callback from inside the chip's own
    /// `clicked`, and the window answered by rebuilding the chips: the
    /// chip being clicked was taken away while GTK was still in its
    /// handler. Now a click only queues a message; the callback runs after
    /// the update has returned, and the chips it asks for, and a click
    /// fired from inside it, are queued too.
    fn clicks_are_queued() {
        let dock = dock();
        let heard: Rc<RefCell<Vec<(AssemblyId, usize)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&dock);
        dock.connect_outputs(move |DockOutput::Move(assembly)| {
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
}
