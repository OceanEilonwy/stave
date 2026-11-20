//! The 3D view's floating controls on the view: each placed where
//! `view_layout` puts it, and the tool groups (scene menu, camera column,
//! chips, motion slider) docked by pressing and holding one, then
//! dragging it. While a group is dragged it follows the pointer and the
//! spots it may land in show as dashed outlines, the nearest one
//! brighter; let go, it docks there, lying flat in a corner or standing
//! upright on the right edge. There is no handle to grab: the press and
//! hold is the way in. Which spot each group is in belongs to the panel
//! layout (`workbench`), never to a `.stave` file.
//!
//! The docking is a relm4 component, [`FloatsComponent`]: the groups'
//! spots and the drag in progress are its model, only its update changes
//! them, and the gestures only send [`FloatsInput`] messages. Its root is
//! the view's overlay, the 3D view under the controls. Each control
//! floats in a wrapper of the component's own (its *float*), which it
//! places, styles while it is dragged and puts the gestures and the
//! "Press and hold" tooltip on; the controls themselves, other
//! components' roots, are only placed in their floats and never touched
//! (docs/relm4-migration.md, "Accepted exceptions": window-level slots).
//! The overlay asks where each float goes while GTK lays it out, which no
//! message can answer, so after every update the overlay's handler is
//! given a copy of what it needs ([`Placer`]), as the outlines' draw
//! function is. [`Floats`] owns its controller; its outputs go to the
//! owner's input, forwarded when it is made, and the spots come to it as
//! [`FloatsInput::Refresh`] (the spots have one keeper, the workbench,
//! and the floats hold a copy).

use stave_app::i18n::tr;

use super::view_layout::{self, Floating, Group, Rect, Spot, Spots};
use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};

/// The floats, each holding one control, measured by GTK. A float shows
/// while its control does: a hidden control measures nothing, so its
/// float is left out.
#[derive(Clone)]
struct Measured(Vec<(Floating, adw::Bin)>);

impl Measured {
    fn widget(&self, item: Floating) -> Option<&adw::Bin> {
        self.0.iter().find(|(i, _)| *i == item).map(|(_, w)| w)
    }

    fn item_of(&self, widget: &gtk::Widget) -> Option<Floating> {
        self.0
            .iter()
            .find(|(_, w)| w.upcast_ref::<gtk::Widget>() == widget)
            .map(|(i, _)| *i)
    }

    /// `true` while a group's control is showing (its own component shows
    /// or hides it).
    fn shows(&self, group: Group) -> bool {
        self.widget(group.floating())
            .and_then(adw::Bin::child)
            .is_some_and(|control| control.is_visible())
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
#[derive(Clone, Debug, PartialEq)]
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

/// Where the controls go, as of the last update: the overlay's
/// `get-child-position` handler keeps this copy until the next update
/// hands it a new one, so it shares nothing with the model.
#[derive(Clone)]
struct Placer {
    measured: Measured,
    spots: Spots,
    /// The group being dragged and where it is drawn as it follows the
    /// pointer.
    dragged: Option<(Group, Rect)>,
}

impl Placer {
    /// Where a control goes now.
    fn rect_of(&self, figure: &gtk::Overlay, item: Floating) -> gdk::Rectangle {
        let placement =
            view_layout::place(figure.width(), figure.height(), &self.measured, &self.spots);
        // A control left out (hidden, or no room) takes no room either,
        // so it can't catch the pointer meant for the view.
        let mut rect = placement.get(item).unwrap_or(Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        });
        if let Some((group, shown_at)) = self.dragged
            && Some(group) == Group::of(item)
        {
            rect = Rect {
                width: rect.width,
                height: rect.height,
                ..shown_at
            };
        }
        gdk::Rectangle::new(rect.x, rect.y, rect.width, rect.height)
    }
}

/// The docking's state: the spot each tool group is in, and the drag in
/// progress.
#[derive(Debug)]
pub struct FloatsComponent {
    spots: Spots,
    drag: Option<Drag>,
}

/// What the floating controls are made with.
pub struct FloatsInit {
    /// The 3D view, under the controls: placed as the overlay's child.
    pub view: gtk::Widget,
    /// The controls, each with what it is: each placed in a float of the
    /// component's own, and otherwise left as its own component has it.
    pub floating: Vec<(Floating, gtk::Widget)>,
}

/// What the floating controls are told.
#[derive(Debug)]
pub enum FloatsInput {
    /// The tool groups are in these spots now (a layout applied or
    /// reset): lay each out for its spot and place them again.
    Refresh(Spots),
    /// People held a group long enough to drag it, pressing here inside
    /// it.
    Held {
        /// The group.
        group: Group,
        /// Where the press was, from the group's left edge.
        x: f64,
        /// Where the press was, from the group's top edge.
        y: f64,
    },
    /// People moved the pointer while dragging, this far from the press.
    Moved {
        /// Across.
        x: f64,
        /// Down.
        y: f64,
    },
    /// People let go of the group (or the drag was cancelled).
    LetGo,
}

/// What the floating controls tell their owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatsOutput {
    /// Lay this group out flat or upright.
    Orient {
        /// The group.
        group: Group,
        /// Flat (horizontal) or upright (vertical).
        orientation: gtk::Orientation,
    },
    /// People docked a group in a new spot.
    Docked {
        /// The group.
        group: Group,
        /// Where it went.
        spot: Spot,
    },
}

/// The docking's widgets.
pub struct FloatsWidgets {
    /// The view's overlay (the component's root).
    figure: gtk::Overlay,
    /// The dashed outlines, under the controls.
    outlines: gtk::DrawingArea,
    /// The floats, one per control.
    measured: Measured,
    /// The overlay's `get-child-position` handler, holding the [`Placer`]
    /// copied at the last update.
    placing: Option<glib::SignalHandlerId>,
    /// The press-and-hold and drag controllers put on each group's float.
    gestures: Vec<(adw::Bin, gtk::EventController)>,
}

impl FloatsComponent {
    /// Hands the overlay and the outlines a copy of where things are now.
    fn show(&self, widgets: &mut FloatsWidgets) {
        if let Some(handler) = widgets.placing.take() {
            widgets.figure.disconnect(handler);
        }
        let placer = Placer {
            measured: widgets.measured.clone(),
            spots: self.spots.clone(),
            dragged: self.drag.as_ref().map(|drag| (drag.group, drag.shown_at())),
        };
        widgets.placing = Some(
            widgets
                .figure
                .connect_get_child_position(move |figure, widget| {
                    let item = placer.measured.item_of(widget)?;
                    Some(placer.rect_of(figure, item))
                }),
        );
        let outlines = self
            .drag
            .as_ref()
            .map(|drag| (drag.targets.clone(), drag.nearest));
        widgets.outlines.set_draw_func(move |area, cairo, _, _| {
            if let Some((targets, nearest)) = &outlines {
                draw_outlines(area, cairo, targets, *nearest);
            }
        });
    }

    /// Lays every group out for its spot and places them again.
    fn refresh(&self, figure: &gtk::Overlay, sender: &ComponentSender<Self>) {
        for group in Group::ALL {
            // Nobody listening is not a mistake.
            let _ = sender.output(FloatsOutput::Orient {
                group,
                orientation: orientation(self.spots.spot_of(group)),
            });
        }
        figure.queue_allocate();
    }

    /// A group was held long enough: it follows the pointer, and the
    /// spots it may land in show.
    fn start(&mut self, widgets: &FloatsWidgets, group: Group, press: (f64, f64)) {
        let figure = &widgets.figure;
        let measured = &widgets.measured;
        let (width, height) = (figure.width(), figure.height());
        let placement = view_layout::place(width, height, measured, &self.spots);
        let Some(from) = placement.get(group.floating()) else {
            return;
        };
        let upright_now = self.spots.spot_of(group).is_upright();
        let targets = self
            .spots
            .free_for(group, |other| measured.shows(other))
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
        self.drag = Some(Drag {
            group,
            from,
            press,
            moved: (0.0, 0.0),
            targets,
            nearest: None,
        });
        if let Some(widget) = measured.widget(group.floating()) {
            widget.add_css_class("stave-dragging");
        }
        figure.set_cursor_from_name(Some("grabbing"));
        figure.queue_allocate();
        widgets.outlines.queue_draw();
    }

    /// The pointer moved during a drag.
    fn follow(&mut self, widgets: &FloatsWidgets, moved: (f64, f64)) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        drag.moved = moved;
        drag.nearest = view_layout::nearest(drag.pointer(), &drag.targets);
        widgets.figure.queue_allocate();
        widgets.outlines.queue_draw();
    }

    /// The group was let go: the owner is told the nearest free spot,
    /// for the keeper of the spots to dock it there, and the group moves
    /// when the keeper's [`FloatsInput::Refresh`] comes back; with no
    /// free spot it goes back where it was.
    fn finish(&mut self, widgets: &FloatsWidgets, sender: &ComponentSender<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if let Some(widget) = widgets.measured.widget(drag.group.floating()) {
            widget.remove_css_class("stave-dragging");
        }
        widgets.figure.set_cursor(None);
        match view_layout::nearest(drag.pointer(), &drag.targets) {
            Some(spot) => {
                // The spots are the workbench's: this copy changes only
                // with its `Refresh`.
                let _ = sender.output(FloatsOutput::Docked {
                    group: drag.group,
                    spot,
                });
                widgets.figure.queue_allocate();
            }
            None => self.refresh(&widgets.figure, sender),
        }
        widgets.outlines.queue_draw();
    }
}

impl Component for FloatsComponent {
    type Init = FloatsInit;
    type Input = FloatsInput;
    type Output = FloatsOutput;
    type CommandOutput = ();
    type Root = gtk::Overlay;
    type Widgets = FloatsWidgets;

    fn init_root() -> gtk::Overlay {
        gtk::Overlay::builder()
            .vexpand(true)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["stave-viewport", "stave-figure"])
            .build()
    }

    fn init(
        init: FloatsInit,
        root: gtk::Overlay,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let FloatsInit { view, floating } = init;
        root.set_child(Some(&view));
        // The dashed outlines, under the controls; they never catch the
        // pointer.
        let outlines = gtk::DrawingArea::builder()
            .can_target(false)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["stave-dock-targets"])
            .build();
        root.add_overlay(&outlines);
        let floats = floating
            .into_iter()
            .map(|(item, control)| {
                // The layout gives each float its exact place; the control
                // fills it.
                let float = adw::Bin::builder().child(&control).build();
                root.add_overlay(&float);
                (item, float)
            })
            .collect();
        let measured = Measured(floats);
        let gestures = Group::ALL
            .into_iter()
            .filter_map(|group| {
                let float = measured.widget(group.floating())?;
                Some(make_draggable(group, float, sender.input_sender()))
            })
            .flatten()
            .collect();
        // The spots come with the owner's first `Refresh`.
        let model = FloatsComponent {
            spots: Spots::default(),
            drag: None,
        };
        let mut widgets = FloatsWidgets {
            figure: root,
            outlines,
            measured,
            placing: None,
            gestures,
        };
        model.show(&mut widgets);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut FloatsWidgets,
        message: FloatsInput,
        sender: ComponentSender<Self>,
        _root: &gtk::Overlay,
    ) {
        match message {
            FloatsInput::Refresh(spots) => {
                self.spots = spots;
                self.refresh(&widgets.figure, &sender);
            }
            FloatsInput::Held { group, x, y } => self.start(widgets, group, (x, y)),
            FloatsInput::Moved { x, y } => self.follow(widgets, (x, y)),
            FloatsInput::LetGo => self.finish(widgets, &sender),
        }
        self.show(widgets);
    }

    /// Takes the component's handlers, gestures and outlines off its
    /// overlay and floats, which live on while the window holds them.
    fn shutdown(&mut self, widgets: &mut FloatsWidgets, _output: relm4::Sender<FloatsOutput>) {
        if let Some(handler) = widgets.placing.take() {
            widgets.figure.disconnect(handler);
        }
        for (float, controller) in widgets.gestures.drain(..) {
            float.remove_controller(&controller);
        }
        widgets.figure.remove_overlay(&widgets.outlines);
    }
}

/// Press and hold a group, then drag it: the gestures put on its float,
/// and the float's tooltip saying so (a control without a tooltip of its
/// own shows it).
fn make_draggable(
    group: Group,
    widget: &adw::Bin,
    input: &relm4::Sender<FloatsInput>,
) -> [(adw::Bin, gtk::EventController); 2] {
    let hold = gtk::GestureLongPress::builder()
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let drag = gtk::GestureDrag::builder()
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    hold.group_with(&drag);
    let sent = input.clone();
    hold.connect_pressed(move |hold, x, y| {
        // The buttons under the press let go of it: this is a drag.
        hold.set_state(gtk::EventSequenceState::Claimed);
        sent.emit(FloatsInput::Held { group, x, y });
    });
    let sent = input.clone();
    drag.connect_drag_update(move |_, x, y| sent.emit(FloatsInput::Moved { x, y }));
    let sent = input.clone();
    drag.connect_drag_end(move |_, _, _| sent.emit(FloatsInput::LetGo));
    let sent = input.clone();
    drag.connect_cancel(move |_, _| sent.emit(FloatsInput::LetGo));
    widget.add_controller(hold.clone());
    widget.add_controller(drag.clone());
    widget.set_tooltip_text(Some(&tr("Press and hold to move it")));
    [
        (widget.clone(), hold.upcast()),
        (widget.clone(), drag.upcast()),
    ]
}

/// The controls floating on the view: it owns the [`FloatsComponent`]
/// (dropping it shuts the component down and takes its handlers off the
/// view); its [`FloatsOutput`]s go to the owner's input.
pub struct Floats {
    /// The 3D view with the controls floating on it, for the owner to
    /// place.
    pub root: gtk::Overlay,
    controller: Controller<FloatsComponent>,
}

impl Floats {
    /// Floats the controls on the 3D view (`view`), each where
    /// `view_layout` puts it for the view's size, what is showing and the
    /// groups' spots, so none covers another. The spots come with
    /// [`Floats::refresh`]. Its outputs (a group to lay flat or upright,
    /// a group people docked) go to `owner` as `message(output)`.
    pub fn new<M: 'static>(
        view: &impl IsA<gtk::Widget>,
        floating: Vec<(Floating, gtk::Widget)>,
        owner: &relm4::Sender<M>,
        message: impl Fn(FloatsOutput) -> M + 'static,
    ) -> Floats {
        let controller = FloatsComponent::builder()
            .launch(FloatsInit {
                view: view.clone().upcast(),
                floating,
            })
            .forward(owner, message);
        Floats {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Lays every group out for its spot (telling the owner how to lay
    /// each one, [`FloatsOutput::Orient`]) and places them again: once
    /// the workbench is made, and after a layout is applied or reset, or a
    /// group docked. The workbench keeps the spots and says where they
    /// are.
    pub fn refresh(&self, spots: Spots) {
        self.send(FloatsInput::Refresh(spots));
    }

    /// Queues `input`; once the floats have shut down there is nobody to
    /// tell.
    fn send(&self, input: FloatsInput) {
        let _ = self.controller.sender().send(input);
    }
}

/// The dashed outlines of the spots a dragged group may land in, the
/// nearest brighter.
fn draw_outlines(
    area: &gtk::DrawingArea,
    cairo: &gtk::cairo::Context,
    targets: &[(Spot, Rect)],
    nearest: Option<Spot>,
) {
    let colour = area.color();
    for (spot, rect) in targets {
        let near = nearest == Some(*spot);
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
    use std::cell::{Cell, RefCell};
    use std::rc::{Rc, Weak};

    use super::*;
    use crate::ui::gtk_test::{handle_each, on_gtk_thread, recorder, settle};
    use crate::ui::workbench::{Layout, PanelId, Remember, Workbench, WorkbenchOutput};

    /// A workbench keeping the tool groups' spots, as the window's does,
    /// every panel a label; nothing is written. Its outputs go to `owner`.
    fn workbench_for(owner: &relm4::Sender<WorkbenchOutput>) -> Workbench {
        let contents = PanelId::ALL
            .into_iter()
            .map(|id| (id, gtk::Label::new(Some(&id.title())).upcast()))
            .collect();
        let Ok(workbench) =
            Workbench::new(contents, Remember::Nothing, owner, std::convert::identity)
        else {
            panic!("the dock is built");
        };
        workbench
    }

    /// A workbench whose outputs nobody hears.
    fn workbench() -> Workbench {
        let (owner, _) = recorder();
        workbench_for(&owner)
    }

    /// Floats on a blank view whose outputs go to `owner`, given the
    /// groups' `spots` as the window gives them once its workbench is made.
    fn floats_for(
        floating: Vec<(Floating, gtk::Widget)>,
        spots: Spots,
        owner: &relm4::Sender<FloatsOutput>,
    ) -> Floats {
        let floats = Floats::new(
            &gtk::DrawingArea::new(),
            floating,
            owner,
            std::convert::identity,
        );
        floats.refresh(spots);
        floats
    }

    /// A control's float: the wrapper the floats placed it in.
    fn float_of(control: &gtk::Widget) -> gtk::Widget {
        control.parent().expect("the control is in its float")
    }

    /// The dashed outlines in the floats' overlay.
    fn outlines_of(floats: &Floats) -> Option<gtk::Widget> {
        std::iter::successors(floats.root.first_child(), gtk::Widget::next_sibling)
            .find(|child| child.has_css_class("stave-dock-targets"))
    }

    /// A blank control of this size.
    fn block(width: i32, height: i32) -> gtk::Widget {
        gtk::Box::builder()
            .width_request(width)
            .height_request(height)
            .build()
            .upcast::<gtk::Widget>()
    }

    /// The four groups' controls, the chips hidden.
    fn controls() -> Vec<(Floating, gtk::Widget)> {
        let moves = block(300, 36);
        moves.set_visible(false);
        vec![
            (Floating::Scene, block(120, 40)),
            (Floating::Camera, block(40, 200)),
            (Floating::Moves, moves),
            (Floating::Motion, block(300, 40)),
        ]
    }

    /// The floats' overlay, in a window big enough to lay the controls
    /// out.
    fn in_a_window(floats: &Floats) -> gtk::Window {
        gtk::Window::builder()
            .default_width(1000)
            .default_height(700)
            .child(&floats.root)
            .build()
    }

    /// Lets GTK lay the window out.
    fn lay_out(window: &gtk::Window) {
        window.present();
        let context = gtk::glib::MainContext::default();
        for _ in 0..50 {
            context.iteration(false);
        }
        settle();
    }

    #[test]
    fn a_held_group_dragged_to_a_free_corner_docks_there() {
        on_gtk_thread(drag_to_a_corner);
    }

    /// The camera column held and dragged to the bottom left corner, where
    /// no group shows: the layout (the workbench, the spots' one keeper)
    /// hears of it and docks it there, and the floats, refreshed by it,
    /// lay it flat, the chips (hidden) taking the right edge.
    fn drag_to_a_corner() {
        let laid: Rc<RefCell<Vec<(Group, gtk::Orientation)>>> = Rc::default();
        let docked = Rc::new(Cell::new(None));
        let laid_by = laid.clone();
        let docked_by = docked.clone();
        let (workbench_owner, workbench_outputs) = relm4::channel();
        let workbench = Rc::new(workbench_for(&workbench_owner));
        let layout = Rc::downgrade(&workbench);
        let (owner, outputs) = relm4::channel();
        let floats = Rc::new(floats_for(controls(), workbench.tool_spots(), &owner));
        let window = in_a_window(&floats);
        let figure = floats.root.clone();
        handle_each(outputs, move |output| match output {
            FloatsOutput::Orient { group, orientation } => {
                laid_by.borrow_mut().push((group, orientation));
            }
            FloatsOutput::Docked { group, spot } => {
                docked_by.set(Some((group, spot)));
                // As the window passes it on.
                if let Some(workbench) = layout.upgrade() {
                    workbench.dock_tool(group, spot);
                }
            }
        });
        let refresher = Rc::downgrade(&floats);
        handle_each(workbench_outputs, move |output| {
            if let WorkbenchOutput::ToolsChanged(spots) = output
                && let Some(floats) = refresher.upgrade()
            {
                floats.refresh(spots);
            }
        });
        lay_out(&window);
        floats.send(FloatsInput::Held {
            group: Group::Camera,
            x: 20.0,
            y: 20.0,
        });
        settle();
        let (targets, from) = {
            let model = floats.controller.model();
            let drag = model.drag.as_ref().expect("the drag started");
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
        floats.send(FloatsInput::Moved {
            x: f64::from(pointer.0 - from.x - 20),
            y: f64::from(pointer.1 - from.y - 20),
        });
        floats.send(FloatsInput::LetGo);
        settle();
        assert_eq!(docked.get(), Some((Group::Camera, Spot::BottomLeft)));
        let spots = floats.controller.model().spots.clone();
        assert_eq!(spots.spot_of(Group::Camera), Spot::BottomLeft);
        assert_eq!(spots.spot_of(Group::Moves), Spot::Right);
        assert!(
            laid.borrow()
                .contains(&(Group::Camera, gtk::Orientation::Horizontal))
        );
        assert!(floats.controller.model().drag.is_none());
        window.destroy();
    }

    /// The press-and-hold and drag gestures on a control.
    fn gestures_of(
        widget: &gtk::Widget,
    ) -> (Option<gtk::GestureLongPress>, Option<gtk::GestureDrag>) {
        let controllers = widget.observe_controllers();
        let all: Vec<_> = (0..controllers.n_items())
            .filter_map(|index| controllers.item(index))
            .collect();
        let hold = all.iter().find_map(|item| item.clone().downcast().ok());
        let drag = all.iter().find_map(|item| item.clone().downcast().ok());
        (hold, drag)
    }

    #[test]
    fn dropped_floats_are_freed() {
        on_gtk_thread(floats_are_freed);
    }

    /// Dropping the floats shuts the component down when the main loop
    /// next runs: its outlines leave the view, its gestures leave the
    /// floats (which live on with the view the window holds), and the
    /// model and the callbacks are freed.
    fn floats_are_freed() {
        let floating = controls();
        let camera = floating
            .iter()
            .find(|(item, _)| *item == Floating::Camera)
            .map(|(_, widget)| widget.clone())
            .expect("a camera column");
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let workbench = workbench();
        let (owner, _) = recorder();
        let floats = Floats::new(&gtk::DrawingArea::new(), floating, &owner, move |output| {
            let _ = &held;
            output
        });
        floats.refresh(workbench.tool_spots());
        let window = in_a_window(&floats);
        let callbacks = Rc::downgrade(&token);
        drop(token);
        lay_out(&window);
        let outlines = outlines_of(&floats).expect("the outlines").downgrade();
        let input = floats.controller.sender().clone();
        let float = float_of(&camera);
        assert!(matches!(gestures_of(&float), (Some(_), Some(_))));
        assert!(
            gestures_of(&camera) == (None, None),
            "the control itself is left as its component made it"
        );
        drop(floats);
        settle();
        assert!(outlines.upgrade().is_none(), "the outlines are freed");
        assert!(
            matches!(gestures_of(&float), (None, None)),
            "the gestures are taken off the float"
        );
        assert!(
            input.send(FloatsInput::LetGo).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callbacks.upgrade().is_none(), "the callbacks are freed");
        window.destroy();
    }

    #[test]
    fn a_signal_fired_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(signals_are_queued);
    }

    /// The old floats ran a drag's steps inside GTK's signals, borrowing
    /// the drag as they went. Now the gestures only queue messages: the
    /// drag moves only when the main loop runs. The owner hears of the
    /// dock after the update that let the group go (reading the model
    /// there would panic if it had not), and a gesture's signal fired
    /// while it handles that, a dock asked of the workbench there and the
    /// refresh the workbench asks for after it, which moves the group,
    /// are queued too.
    fn signals_are_queued() {
        let floating = controls();
        let camera = floating
            .iter()
            .find(|(item, _)| *item == Floating::Camera)
            .map(|(_, widget)| widget.clone())
            .expect("a camera column");
        let me: Rc<RefCell<Weak<Floats>>> = Rc::default();
        // (the group docked, whether a drag was still on when told)
        let heard: Rc<RefCell<Vec<(Group, bool)>>> = Rc::default();
        let (log, find) = (Rc::clone(&heard), Rc::clone(&me));
        let inner_drag: Rc<RefCell<Option<gtk::GestureDrag>>> = Rc::default();
        let fire = Rc::clone(&inner_drag);
        // The workbench keeps the groups' spots, as the window's does.
        let (workbench_owner, workbench_outputs) = relm4::channel();
        let workbench = Rc::new(workbench_for(&workbench_owner));
        let layout = Rc::downgrade(&workbench);
        let (owner, outputs) = relm4::channel();
        let floats = Rc::new(floats_for(floating, workbench.tool_spots(), &owner));
        let window = in_a_window(&floats);
        handle_each(outputs, move |output| {
            let FloatsOutput::Docked { group, spot } = output else {
                return;
            };
            let Some(floats) = find.borrow().upgrade() else {
                return;
            };
            let dragging = floats.controller.model().drag.is_some();
            // Docking in the workbench is queued; it tells the floats
            // to refresh once it has handled it.
            if let Some(workbench) = layout.upgrade() {
                workbench.dock_tool(group, spot);
            }
            // GTK ending the drag again from in here only queues a
            // message.
            if let Some(drag) = fire.borrow().as_ref() {
                drag.emit_by_name::<()>("drag-end", &[&0.0_f64, &0.0_f64]);
            }
            log.borrow_mut().push((group, dragging));
        });
        me.replace(Rc::downgrade(&floats));
        // As the window wires them: a refresh asked for when the workbench
        // says the groups moved, itself queued.
        let refresher = Rc::downgrade(&floats);
        handle_each(workbench_outputs, move |output| {
            if let WorkbenchOutput::ToolsChanged(spots) = output
                && let Some(floats) = refresher.upgrade()
            {
                floats.refresh(spots);
            }
        });
        lay_out(&window);
        let (Some(hold), Some(drag)) = gestures_of(&float_of(&camera)) else {
            panic!("the camera column can be held and dragged");
        };
        inner_drag.replace(Some(drag.clone()));

        // Held, then dragged far down and to the left, then let go: all
        // only queued.
        hold.emit_by_name::<()>("pressed", &[&20.0_f64, &20.0_f64]);
        assert!(
            floats.controller.model().drag.is_none(),
            "the hold is only queued"
        );
        settle();
        assert!(
            floats.controller.model().drag.is_some(),
            "then the drag starts"
        );
        let far = (-2000.0_f64, 2000.0_f64);
        drag.emit_by_name::<()>("drag-update", &[&far.0, &far.1]);
        drag.emit_by_name::<()>("drag-end", &[&far.0, &far.1]);
        assert_eq!(
            floats
                .controller
                .model()
                .drag
                .as_ref()
                .map(|drag| drag.moved),
            Some((0.0, 0.0)),
            "the move is only queued"
        );
        assert!(heard.borrow().is_empty());
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![(Group::Camera, false)],
            "docked once, told after the update, the inner drag-end a no-op"
        );
        assert_eq!(
            floats.controller.model().spots.spot_of(Group::Camera),
            Spot::BottomLeft
        );
        assert_eq!(
            workbench.layout().tools,
            floats.controller.model().spots,
            "the workbench keeps where the group went"
        );
        window.destroy();
    }

    #[test]
    fn every_group_is_oriented_for_its_spot_once_the_owner_listens() {
        on_gtk_thread(oriented_at_once);
    }

    /// The floats tell their owner to lay each group out for its spot, from
    /// the spots the workbench keeps, as soon as the owner gives them:
    /// before the window's first frame.
    fn oriented_at_once() {
        let workbench = workbench();
        let mut layout = Layout::default();
        layout.tools.dock(Group::Scene, Spot::Right);
        workbench.apply(layout);
        settle();
        let (owner, heard) = recorder();
        let floats = floats_for(controls(), workbench.tool_spots(), &owner);
        let window = in_a_window(&floats);
        settle();
        let laid: Vec<(Group, gtk::Orientation)> = heard
            .borrow()
            .iter()
            .filter_map(|output| match *output {
                FloatsOutput::Orient { group, orientation } => Some((group, orientation)),
                FloatsOutput::Docked { .. } => None,
            })
            .collect();
        let groups: Vec<Group> = laid.iter().map(|(group, _)| *group).collect();
        assert_eq!(groups, Group::ALL.to_vec(), "every group, once");
        assert!(
            laid.contains(&(Group::Scene, gtk::Orientation::Vertical)),
            "the scene menu stands upright on the right edge"
        );
        drop(floats);
        settle();
        window.destroy();
    }

    #[test]
    fn a_reset_and_a_dock_in_one_turn_leave_the_floats_where_the_workbench_is() {
        on_gtk_thread(floats_follow_reset_then_dock);
    }

    /// Wired as the window wires them: a reset and a dock asked for in
    /// the same turn of the main loop. The floats end where the workbench
    /// keeps (and saves) the groups, never at an older copy.
    fn floats_follow_reset_then_dock() {
        let (workbench_owner, workbench_outputs) = relm4::channel();
        let workbench = workbench_for(&workbench_owner);
        let mut layout = Layout::default();
        layout.tools.dock(Group::Scene, Spot::Right);
        workbench.apply(layout);
        settle();
        let (owner, _) = recorder();
        let floats = Rc::new(floats_for(controls(), workbench.tool_spots(), &owner));
        let window = in_a_window(&floats);
        let refresher = Rc::downgrade(&floats);
        handle_each(workbench_outputs, move |output| {
            if let WorkbenchOutput::ToolsChanged(spots) = output
                && let Some(floats) = refresher.upgrade()
            {
                floats.refresh(spots);
            }
        });
        lay_out(&window);
        workbench.reset();
        workbench.dock_tool(Group::Motion, Spot::TopLeft);
        settle();
        let mut expected = Spots::default();
        expected.dock(Group::Motion, Spot::TopLeft);
        assert_eq!(workbench.layout().tools, expected);
        assert_eq!(floats.controller.model().spots, expected);
        window.destroy();
    }
}
