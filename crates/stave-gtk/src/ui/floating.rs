//! The 3D view's floating controls on the view: each placed where
//! `view_layout` puts it, and the tool groups (scene menu, camera column,
//! chips, motion slider) docked by pressing and holding one, then
//! dragging it. While a group is dragged it follows the pointer and the
//! spots it may land in show as dashed outlines, the nearest one
//! brighter; let go, it docks there, lying flat in a corner or standing
//! upright on the right edge. There is no handle to grab: the press and
//! hold is the way in. Which spot each group is in belongs to the panel
//! layout (`workbench`), never to a `.stave` file.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;

use super::view_layout::{self, Floating, Group, Rect, Spot, Spots};

/// The floating controls, measured by GTK.
struct Measured(Vec<(Floating, gtk::Widget)>);

impl Measured {
    fn widget(&self, item: Floating) -> Option<&gtk::Widget> {
        self.0.iter().find(|(i, _)| *i == item).map(|(_, w)| w)
    }

    fn item_of(&self, widget: &gtk::Widget) -> Option<Floating> {
        self.0.iter().find(|(_, w)| w == widget).map(|(i, _)| *i)
    }

    /// `true` while a group's control is showing.
    fn shows(&self, group: Group) -> bool {
        self.widget(group.floating())
            .is_some_and(gtk::prelude::WidgetExt::is_visible)
    }
}

impl view_layout::Measure for Measured {
    fn widths(&self, item: Floating) -> Option<(i32, i32)> {
        let widget = self.widget(item).filter(|w| w.is_visible())?;
        let (smallest, natural, _, _) = widget.measure(gtk::Orientation::Horizontal, -1);
        (natural > 0).then_some((smallest, natural))
    }

    fn height(&self, item: Floating, width: i32) -> i32 {
        self.widget(item).map_or(0, |widget| {
            let (_, natural, _, _) = widget.measure(gtk::Orientation::Vertical, width);
            natural
        })
    }
}

/// A group being dragged.
struct Drag {
    group: Group,
    /// Where it was when the drag started.
    from: Rect,
    /// Where the press was, inside the group.
    press: (f64, f64),
    /// How far the pointer has moved since.
    moved: (f64, f64),
    /// The outlines of the spots it may land in.
    targets: Vec<(Spot, Rect)>,
    /// The one it would land in now.
    nearest: Option<Spot>,
}

impl Drag {
    /// The pointer, in the view's pixels.
    fn pointer(&self) -> (i32, i32) {
        let x = f64::from(self.from.x) + self.press.0 + self.moved.0;
        let y = f64::from(self.from.y) + self.press.1 + self.moved.1;
        (crate::convert::int_of(x), crate::convert::int_of(y))
    }

    /// Where the group is drawn while it follows the pointer.
    fn shown_at(&self) -> Rect {
        let (x, y) = self.pointer();
        Rect {
            x: x - crate::convert::int_of(self.press.0),
            y: y - crate::convert::int_of(self.press.1),
            ..self.from
        }
    }
}

/// Lays a group out flat or upright.
pub type Orient = Box<dyn Fn(Group, gtk::Orientation)>;
/// Told when a group is docked in a spot.
pub type OnDock = Box<dyn Fn(Group, Spot)>;

/// The controls floating on the view.
pub struct Floats {
    figure: gtk::Overlay,
    measured: Measured,
    spots: Rc<RefCell<Spots>>,
    drag: RefCell<Option<Drag>>,
    outlines: gtk::DrawingArea,
    orient: Orient,
    on_dock: OnDock,
}

impl Floats {
    /// Floats the controls on the 3D view, each where `view_layout` puts
    /// it for the view's size, what is showing and the groups' `spots`,
    /// so none covers another. `orient` lays a group flat or upright;
    /// `on_dock` hears when one is dragged to a new spot.
    pub fn new(
        figure: &gtk::Overlay,
        floating: Vec<(Floating, gtk::Widget)>,
        spots: Rc<RefCell<Spots>>,
        orient: Orient,
        on_dock: OnDock,
    ) -> Rc<Floats> {
        // The dashed outlines, under the controls; they never catch the
        // pointer.
        let outlines = gtk::DrawingArea::builder()
            .can_target(false)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["stave-dock-targets"])
            .build();
        figure.add_overlay(&outlines);
        for (_, widget) in &floating {
            // The layout gives each its exact place: no margins of its own.
            widget.set_halign(gtk::Align::Fill);
            widget.set_valign(gtk::Align::Fill);
            widget.set_margin_top(0);
            widget.set_margin_bottom(0);
            widget.set_margin_start(0);
            widget.set_margin_end(0);
            figure.add_overlay(widget);
        }
        let floats = Rc::new(Floats {
            figure: figure.clone(),
            measured: Measured(floating),
            spots,
            drag: RefCell::new(None),
            outlines,
            orient,
            on_dock,
        });
        let weak = Rc::downgrade(&floats);
        figure.connect_get_child_position(move |figure, widget| {
            let floats = weak.upgrade()?;
            let item = floats.measured.item_of(widget)?;
            Some(floats.rect_of(figure, item))
        });
        let weak = Rc::downgrade(&floats);
        floats.outlines.set_draw_func(move |area, cairo, _, _| {
            if let Some(floats) = weak.upgrade() {
                floats.draw_outlines(area, cairo);
            }
        });
        for group in Group::ALL {
            if let Some(widget) = floats.measured.widget(group.floating()).cloned() {
                floats.make_draggable(group, &widget);
            }
        }
        floats.refresh();
        floats
    }

    /// Lays every group out for its spot and places them again (after a
    /// layout is applied or reset).
    pub fn refresh(&self) {
        let spots = self.spots.borrow().clone();
        for group in Group::ALL {
            (self.orient)(group, orientation(spots.spot_of(group)));
        }
        self.figure.queue_allocate();
    }

    /// Where a control goes now.
    fn rect_of(&self, figure: &gtk::Overlay, item: Floating) -> gdk::Rectangle {
        let placement = view_layout::place(
            figure.width(),
            figure.height(),
            &self.measured,
            &self.spots.borrow(),
        );
        // A control left out (hidden, or no room) takes no room either,
        // so it can't catch the pointer meant for the view.
        let mut rect = placement.get(item).unwrap_or(Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        });
        if let Some(drag) = self.drag.borrow().as_ref()
            && Some(drag.group) == Group::of(item)
        {
            rect = Rect {
                width: rect.width,
                height: rect.height,
                ..drag.shown_at()
            };
        }
        gdk::Rectangle::new(rect.x, rect.y, rect.width, rect.height)
    }

    /// Press and hold a group, then drag it.
    fn make_draggable(self: &Rc<Self>, group: Group, widget: &gtk::Widget) {
        let hold = gtk::GestureLongPress::builder()
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let drag = gtk::GestureDrag::builder()
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        hold.group_with(&drag);
        let weak = Rc::downgrade(self);
        hold.connect_pressed(move |hold, x, y| {
            let Some(floats) = weak.upgrade() else { return };
            // The buttons under the press let go of it: this is a drag.
            hold.set_state(gtk::EventSequenceState::Claimed);
            floats.start(group, (x, y));
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_update(move |_, x, y| {
            if let Some(floats) = weak.upgrade() {
                floats.follow((x, y));
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(floats) = weak.upgrade() {
                floats.finish();
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_cancel(move |_, _| {
            if let Some(floats) = weak.upgrade() {
                floats.finish();
            }
        });
        widget.add_controller(hold);
        widget.add_controller(drag);
        widget.set_tooltip_text(Some(
            &widget
                .tooltip_text()
                .map(|tip| format!("{tip}\nPress and hold to move it"))
                .unwrap_or_else(|| "Press and hold to move it".to_owned()),
        ));
    }

    /// A group was held long enough: it follows the pointer, and the
    /// spots it may land in show.
    fn start(&self, group: Group, press: (f64, f64)) {
        let figure = &self.figure;
        let (width, height) = (figure.width(), figure.height());
        let placement = view_layout::place(width, height, &self.measured, &self.spots.borrow());
        let Some(from) = placement.get(group.floating()) else {
            return;
        };
        let spots = self.spots.borrow();
        let upright_now = spots.spot_of(group).is_upright();
        let targets = spots
            .free_for(group, |other| self.measured.shows(other))
            .into_iter()
            .map(|spot| {
                // Its size as it would lie in that spot.
                let size = if spot.is_upright() == upright_now {
                    (from.width, from.height)
                } else {
                    (from.height, from.width)
                };
                (spot, view_layout::target(width, height, spot, size))
            })
            .collect();
        drop(spots);
        self.drag.replace(Some(Drag {
            group,
            from,
            press,
            moved: (0.0, 0.0),
            targets,
            nearest: None,
        }));
        if let Some(widget) = self.measured.widget(group.floating()) {
            widget.add_css_class("stave-dragging");
        }
        figure.set_cursor_from_name(Some("grabbing"));
        self.outlines.queue_draw();
    }

    /// The pointer moved during a drag.
    fn follow(&self, moved: (f64, f64)) {
        let mut drag = self.drag.borrow_mut();
        let Some(drag) = drag.as_mut() else {
            return;
        };
        drag.moved = moved;
        drag.nearest = view_layout::nearest(drag.pointer(), &drag.targets);
        self.figure.queue_allocate();
        self.outlines.queue_draw();
    }

    /// The group was let go: it docks in the nearest free spot (or goes
    /// back where it was if there is none).
    fn finish(&self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let Some(widget) = self.measured.widget(drag.group.floating()) {
            widget.remove_css_class("stave-dragging");
        }
        self.figure.set_cursor(None);
        if let Some(spot) = view_layout::nearest(drag.pointer(), &drag.targets) {
            self.spots.borrow_mut().dock(drag.group, spot);
            (self.on_dock)(drag.group, spot);
        }
        self.refresh();
        self.outlines.queue_draw();
    }

    /// The dashed outlines of the spots a dragged group may land in.
    fn draw_outlines(&self, area: &gtk::DrawingArea, cairo: &gtk::cairo::Context) {
        let drag = self.drag.borrow();
        let Some(drag) = drag.as_ref() else {
            return;
        };
        let colour = area.color();
        for (spot, rect) in &drag.targets {
            let near = drag.nearest == Some(*spot);
            rounded(cairo, *rect, 10.0);
            cairo.set_source_rgba(
                f64::from(colour.red()),
                f64::from(colour.green()),
                f64::from(colour.blue()),
                if near { 0.16 } else { 0.06 },
            );
            let _ = cairo.fill_preserve();
            cairo.set_source_rgba(
                f64::from(colour.red()),
                f64::from(colour.green()),
                f64::from(colour.blue()),
                if near { 0.9 } else { 0.5 },
            );
            cairo.set_line_width(1.5);
            cairo.set_dash(&[6.0, 4.0], 0.0);
            let _ = cairo.stroke();
        }
    }
}

/// A group lies flat in a corner and stands upright on the right edge.
fn orientation(spot: Spot) -> gtk::Orientation {
    if spot.is_upright() {
        gtk::Orientation::Vertical
    } else {
        gtk::Orientation::Horizontal
    }
}

/// A rounded rectangle path, a little inside `rect`.
fn rounded(cairo: &gtk::cairo::Context, rect: Rect, radius: f64) {
    let x = f64::from(rect.x) + 0.75;
    let y = f64::from(rect.y) + 0.75;
    let width = (f64::from(rect.width) - 1.5).max(0.0);
    let height = (f64::from(rect.height) - 1.5).max(0.0);
    let radius = radius.min(width / 2.0).min(height / 2.0);
    let quarter = std::f64::consts::FRAC_PI_2;
    cairo.new_sub_path();
    cairo.arc(x + width - radius, y + radius, radius, -quarter, 0.0);
    cairo.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        quarter,
    );
    cairo.arc(
        x + radius,
        y + height - radius,
        radius,
        quarter,
        2.0 * quarter,
    );
    cairo.arc(x + radius, y + radius, radius, 2.0 * quarter, 3.0 * quarter);
    cairo.close_path();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn a_held_group_dragged_to_a_free_corner_docks_there() {
        crate::ui::gtk_test::on_gtk_thread(drag_to_a_corner);
    }

    /// The camera column held and dragged to the bottom left corner, where
    /// no group shows: it docks there, lying flat, the chips (hidden)
    /// taking the right edge; the layout hears of it.
    fn drag_to_a_corner() {
        let block = |width, height| {
            gtk::Box::builder()
                .width_request(width)
                .height_request(height)
                .build()
                .upcast::<gtk::Widget>()
        };
        let moves = block(300, 36);
        moves.set_visible(false);
        let figure = gtk::Overlay::builder()
            .child(&gtk::DrawingArea::new())
            .build();
        let laid: Rc<RefCell<Vec<(Group, gtk::Orientation)>>> = Rc::default();
        let docked = Rc::new(Cell::new(None));
        let laid_by = laid.clone();
        let docked_by = docked.clone();
        let floats = Floats::new(
            &figure,
            vec![
                (Floating::Scene, block(120, 40)),
                (Floating::Camera, block(40, 200)),
                (Floating::Moves, moves),
                (Floating::Motion, block(300, 40)),
            ],
            Rc::new(RefCell::new(Spots::default())),
            Box::new(move |group, orientation| laid_by.borrow_mut().push((group, orientation))),
            Box::new(move |group, spot| docked_by.set(Some((group, spot)))),
        );
        let window = gtk::Window::builder()
            .default_width(1000)
            .default_height(700)
            .child(&figure)
            .build();
        window.present();
        let context = gtk::glib::MainContext::default();
        for _ in 0..50 {
            context.iteration(false);
        }
        floats.start(Group::Camera, (20.0, 20.0));
        let (targets, from) = {
            let drag = floats.drag.borrow();
            let drag = drag.as_ref().expect("the drag started");
            let targets: Vec<Spot> = drag.targets.iter().map(|(spot, _)| *spot).collect();
            (targets, drag.from)
        };
        assert_eq!(
            targets,
            vec![Spot::BottomLeft],
            "only the chips' corner is free"
        );
        // Down to the bottom left corner.
        let pointer = (60, figure.height() - 60);
        floats.follow((
            f64::from(pointer.0 - from.x - 20),
            f64::from(pointer.1 - from.y - 20),
        ));
        floats.finish();
        assert_eq!(docked.get(), Some((Group::Camera, Spot::BottomLeft)));
        let spots = floats.spots.borrow().clone();
        assert_eq!(spots.spot_of(Group::Camera), Spot::BottomLeft);
        assert_eq!(spots.spot_of(Group::Moves), Spot::Right);
        assert!(
            laid.borrow()
                .contains(&(Group::Camera, gtk::Orientation::Horizontal))
        );
        assert!(floats.drag.borrow().is_none());
        window.destroy();
    }
}
