//! The strip under the 3D view: the scene menu, "Pull apart" (with its
//! play button and slider while on), X-ray, the selection's motion slider,
//! and one chip per moving assembly ("Move: door_l, door_r, drawer[top]")
//! that selects it so its slider shows; the window adds the camera
//! buttons at its end. Chips of assemblies with problems are ringed in the
//! problem's colour.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::problems::Mark;
use stave_model::solved::AssemblyId;

use super::chrome::MotionBar;
use super::view_tools::ViewToolbar;

type MoveCallback = Rc<dyn Fn(AssemblyId)>;

/// The dock's widgets.
pub struct Dock {
    /// The dock, to float over the 3D view.
    pub root: gtk::Box,
    /// See through everything but the selection.
    pub xray: gtk::ToggleButton,
    moves: gtk::Box,
    moves_separator: gtk::Separator,
    on_move: RefCell<Option<MoveCallback>>,
}

fn separator() -> gtk::Separator {
    gtk::Separator::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-dock-separator"])
        .build()
}

impl Dock {
    /// A dock holding the scene menu, the view toolbar and the motion bar
    /// (taken out of their own floating boxes).
    pub fn new(scene: &gtk::MenuButton, toolbar: &ViewToolbar, motion: &MotionBar) -> Rc<Dock> {
        // The toolbar and the motion bar float on their own elsewhere; in
        // the dock they are plain rows.
        toolbar.root.set_css_classes(&["stave-dock-part"]);
        toolbar.root.set_margin_top(0);
        toolbar.root.set_margin_start(0);
        toolbar.root.set_valign(gtk::Align::Center);
        toolbar.root.set_halign(gtk::Align::Fill);
        motion
            .root
            .set_css_classes(&["stave-dock-part", "stave-dock-motion"]);
        motion.root.set_margin_bottom(0);
        motion.root.set_valign(gtk::Align::Center);
        motion.root.set_halign(gtk::Align::Fill);
        motion.root.set_spacing(10);
        let xray = gtk::ToggleButton::builder()
            .label("X-ray")
            .tooltip_text("See through everything but the selection")
            .css_classes(["stave-tool"])
            .build();
        let moves = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .valign(gtk::Align::Center)
            .build();
        let moves_separator = separator();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-dock"])
            .build();
        root.append(scene);
        root.append(&toolbar.root);
        root.append(&xray);
        root.append(&separator());
        root.append(&motion.root);
        root.append(&moves_separator);
        root.append(&moves);
        Rc::new(Dock {
            root,
            xray,
            moves,
            moves_separator,
            on_move: RefCell::new(None),
        })
    }

    /// Calls `callback` with the assembly whose chip was chosen.
    pub fn connect_move(&self, callback: impl Fn(AssemblyId) + 'static) {
        self.on_move.replace(Some(Rc::new(callback)));
    }

    /// The moving assemblies' chips: each one's name and problem mark, the
    /// one whose slider shows marked current.
    pub fn show_moves(
        self: &Rc<Self>,
        moving: &[(AssemblyId, String, Option<Mark>)],
        current: Option<AssemblyId>,
    ) {
        while let Some(child) = self.moves.first_child() {
            self.moves.remove(&child);
        }
        let visible = !moving.is_empty();
        self.moves.set_visible(visible);
        self.moves_separator.set_visible(visible);
        if !visible {
            return;
        }
        self.moves.append(
            &gtk::Label::builder()
                .label("Move:")
                .css_classes(["stave-dock-label"])
                .build(),
        );
        for (id, name, mark) in moving {
            let chip = gtk::Button::builder()
                .label(name)
                .tooltip_text(format!("Open or close {name}"))
                .css_classes(["stave-move"])
                .build();
            match mark {
                Some(Mark::Error | Mark::Involved) => chip.add_css_class("error"),
                Some(Mark::Warning) => chip.add_css_class("warning"),
                None => {}
            }
            if current == Some(*id) {
                chip.add_css_class("current");
            }
            let dock = Rc::downgrade(self);
            let id = *id;
            chip.connect_clicked(move |_| {
                let Some(dock) = dock.upgrade() else { return };
                let callback = dock.on_move.borrow().clone();
                if let Some(callback) = callback {
                    callback(id);
                }
            });
            self.moves.append(&chip);
        }
    }
}
