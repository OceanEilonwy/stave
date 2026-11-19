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
            // The tree's mark beside the name: a round dot for an
            // error, a square one for a warning, so the chip's ring is
            // not the only sign.
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            content.append(&super::style::mark_dot(*mark));
            content.append(&gtk::Label::new(Some(name)));
            let chip = gtk::Button::builder()
                .child(&content)
                .tooltip_text(format!("Open or close {name}"))
                .css_classes(["stave-move"])
                .build();
            let problem = match mark {
                Some(Mark::Error) => Some(("error", "Has an error")),
                Some(Mark::Involved) => Some(("error", "Is part of an error")),
                Some(Mark::Warning) => Some(("warning", "Has a warning")),
                None => None,
            };
            if let Some((class, words)) = problem {
                chip.add_css_class(class);
                chip.update_property(&[gtk::accessible::Property::Description(words)]);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_shows_its_problem_by_shape_too() {
        crate::ui::gtk_test::on_gtk_thread(chips_show_marks);
    }

    /// Each chip with a problem carries the tree's mark (round for an
    /// error, square for a warning) and says the problem to a screen
    /// reader; a chip with none shows no mark.
    fn chips_show_marks() {
        let scene = gtk::MenuButton::new();
        let motion = MotionBar::new();
        let dock = Dock::new(&scene, &motion);
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
        let mut marks = Vec::new();
        let mut chip = dock.chips.first_child();
        while let Some(widget) = chip {
            chip = widget.next_sibling();
            let dot = widget
                .first_child()
                .and_then(|content| content.first_child())
                .expect("the chip's mark");
            assert!(dot.has_css_class("stave-mark"));
            marks.push((
                dot.is_visible(),
                dot.has_css_class("error"),
                dot.has_css_class("warning"),
                gtk::test_accessible_has_property(&widget, gtk::AccessibleProperty::Description),
            ));
        }
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
