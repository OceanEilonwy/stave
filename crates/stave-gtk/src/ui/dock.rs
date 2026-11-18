//! The 3D view's floating controls that aren't buttons on the camera
//! column: the scene menu at the top left, the selection's motion slider
//! at the bottom right, and one chip per moving assembly ("Move: door_l,
//! door_r, drawer[top]") at the bottom left, which selects it so its
//! slider shows. Chips of assemblies with problems are ringed in the
//! problem's colour. They step aside while a problem is posed or a joint
//! is in focus.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::problems::Mark;
use stave_model::solved::AssemblyId;

use super::chrome::MotionBar;

type MoveCallback = Rc<dyn Fn(AssemblyId)>;

/// The floating controls.
pub struct Dock {
    /// The scene menu, in its floating box (top left).
    pub scene: gtk::Box,
    /// The moving assemblies' chips (bottom left).
    pub moves: gtk::Box,
    chips: adw::WrapBox,
    /// There are chips to show.
    has_moves: Cell<bool>,
    /// The controls are offered (not while a problem is posed or a joint
    /// is in focus).
    offered: Cell<bool>,
    on_move: RefCell<Option<MoveCallback>>,
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
        let chips = adw::WrapBox::builder()
            .child_spacing(6)
            .line_spacing(6)
            .build();
        let moves = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["stave-dock", "stave-moves"])
            .name("moves")
            .visible(false)
            .build();
        moves.append(
            &gtk::Label::builder()
                .label("Move:")
                .valign(gtk::Align::Start)
                .css_classes(["stave-dock-label"])
                .build(),
        );
        moves.append(&chips);
        Rc::new(Dock {
            scene: scene_box,
            moves,
            chips,
            has_moves: Cell::new(false),
            offered: Cell::new(true),
            on_move: RefCell::new(None),
        })
    }

    /// Calls `callback` with the assembly whose chip was chosen.
    pub fn connect_move(&self, callback: impl Fn(AssemblyId) + 'static) {
        self.on_move.replace(Some(Rc::new(callback)));
    }

    /// Offers the controls, or takes them away while a problem is posed
    /// or a joint is in focus.
    pub fn offer(&self, offered: bool) {
        self.offered.set(offered);
        self.scene.set_visible(offered);
        self.moves.set_visible(offered && self.has_moves.get());
    }

    /// The moving assemblies' chips: each one's name and problem mark, the
    /// one whose slider shows marked current.
    pub fn show_moves(
        self: &Rc<Self>,
        moving: &[(AssemblyId, String, Option<Mark>)],
        current: Option<AssemblyId>,
    ) {
        let wrap = &self.chips;
        while let Some(child) = wrap.first_child() {
            wrap.remove(&child);
        }
        let visible = !moving.is_empty();
        self.has_moves.set(visible);
        self.moves.set_visible(visible && self.offered.get());
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
            wrap.append(&chip);
        }
    }
}
