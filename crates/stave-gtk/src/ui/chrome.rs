//! The smaller pieces of the window: the motion bar ("Open it", floating
//! at the bottom right of the 3D view) and the Cut list's placeholder page.
//!
//! The motion bar is a relm4 component, [`MotionBarComponent`]
//! (docs/relm4-migration.md): what it shows is its model, only its
//! update changes it, and the slider's handler only sends a message. The
//! window sets it with [`MotionBarInput::Show`], which moves the slider
//! with its handler blocked, and hears only people's moves as
//! [`MotionBarOutput::Moved`], forwarded to its input when the bar is
//! made.

use adw::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::i18n::{fill_in, tr};

/// What the motion bar shows: a name, words at each end and the slider's
/// range and value.
#[derive(Debug, Clone, PartialEq)]
pub struct MotionShown {
    /// What moves.
    pub name: String,
    /// The words at the slider's start ("closed").
    pub start: String,
    /// The words at its end ("110°").
    pub end: String,
    /// The slider's end, from 0.
    pub full: f64,
    /// Where the slider is.
    pub value: f64,
}

/// The motion bar's state: what it shows, if anything, and which way it
/// lies.
#[derive(Debug)]
pub struct MotionBarComponent {
    /// What it shows; `None` hides it.
    shown: Option<MotionShown>,
    /// A row (docked in a corner) or upright (docked on the right edge).
    orientation: gtk::Orientation,
}

/// What the motion bar is told.
#[derive(Debug)]
pub enum MotionBarInput {
    /// Show this, or hide the bar: the slider moves without telling
    /// anyone.
    Show(Option<MotionShown>),
    /// Lay the bar out in a row or upright, its slider turned to match.
    SetOrientation(gtk::Orientation),
    /// People moved the slider to this value.
    Moved(f64),
}

/// What the motion bar tells its owner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionBarOutput {
    /// People moved the slider to this value.
    Moved(f64),
}

#[relm4::component(pub)]
impl Component for MotionBarComponent {
    type Init = ();
    type Input = MotionBarInput;
    type Output = MotionBarOutput;
    type CommandOutput = ();

    view! {
        gtk::Box {
            add_css_class: "stave-floating",
            // It floats on the 3D view, placed exactly by the floats.
            #[watch]
            set_orientation: model.orientation,
            #[watch]
            set_visible: model.shown.is_some(),

            #[name = "name"]
            gtk::Label {
                add_css_class: "heading",
            },
            #[name = "start"]
            gtk::Label {
                add_css_class: "stave-key",
            },
            #[name = "scale"]
            gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1) {
                set_draw_value: false,
                #[watch]
                set_orientation: model.orientation,
                #[watch]
                set_width_request: match model.orientation {
                    gtk::Orientation::Vertical => -1,
                    _ => 220,
                },
                #[watch]
                set_height_request: match model.orientation {
                    gtk::Orientation::Vertical => 160,
                    _ => -1,
                },
                connect_value_changed[sender] => move |scale| {
                    sender.input_sender().emit(MotionBarInput::Moved(scale.value()));
                } @moved,
            },
            #[name = "end"]
            gtk::Label {
                add_css_class: "stave-key",
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = MotionBarComponent {
            shown: None,
            orientation: gtk::Orientation::Horizontal,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: MotionBarInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MotionBarInput::Show(shown) => {
                if let Some(shown) = &shown {
                    widgets.name.set_label(&shown.name);
                    widgets.start.set_label(&shown.start);
                    widgets.end.set_label(&shown.end);
                    // Changing the range can move the value on the way:
                    // neither is heard back.
                    let scale = &widgets.scale;
                    scale.block_signal(&widgets.moved);
                    scale.set_range(0.0, shown.full.max(0.1));
                    scale.set_value(shown.value);
                    scale.unblock_signal(&widgets.moved);
                    scale.update_property(&[gtk::accessible::Property::Label(&fill_in(
                        &tr("Open {name}"),
                        &[("name", &shown.name)],
                    ))]);
                }
                self.shown = shown;
            }
            MotionBarInput::SetOrientation(orientation) => self.orientation = orientation,
            MotionBarInput::Moved(value) => {
                // Nobody listening is not a mistake.
                let _ = sender.output(MotionBarOutput::Moved(value));
            }
        }
        self.update_view(widgets, sender);
    }
}

/// The floating bar at the bottom right of the 3D view: a name, words at
/// each end and a slider ("door_l  closed ——— 110°"). It owns the
/// [`MotionBarComponent`]: dropping it shuts the component down. Setters
/// are queued; people's moves go to the owner as the message it chose.
pub struct MotionBar {
    /// The bar, to overlay on the view.
    pub root: gtk::Box,
    controller: Controller<MotionBarComponent>,
}

impl MotionBar {
    /// A hidden bar, whose outputs go to `owner` as `message(output)`.
    pub fn new<M: 'static>(
        owner: &relm4::Sender<M>,
        message: impl Fn(MotionBarOutput) -> M + 'static,
    ) -> MotionBar {
        let controller = MotionBarComponent::builder()
            .launch(())
            .forward(owner, message);
        MotionBar {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Queues `input`; once the bar has shut down there is nobody to tell.
    fn send(&self, input: MotionBarInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Lays the bar out in a row (docked in a corner) or standing upright
    /// (docked on the right edge), its slider turned to match.
    pub fn set_orientation(&self, orientation: gtk::Orientation) {
        self.send(MotionBarInput::SetOrientation(orientation));
    }

    /// Shows the bar, or hides it with `None`, without calling back.
    pub fn show(&self, shown: Option<MotionShown>) {
        self.send(MotionBarInput::Show(shown));
    }
}

/// The Cut list view, until it is built.
pub fn cut_list_page() -> adw::StatusPage {
    adw::StatusPage::builder()
        .title(tr("Cut list is coming"))
        .description(tr("Every board to cut, sized and rounded once, will be listed here. It isn't part of this version yet."))
        .css_classes(["stave-placeholder"])
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, recorder, settle};

    /// The bar for a door, 0 to 110 at `value`.
    fn door(value: f64) -> Option<MotionShown> {
        Some(MotionShown {
            name: "door_l".to_owned(),
            start: "closed".to_owned(),
            end: "110°".to_owned(),
            full: 110.0,
            value,
        })
    }

    /// The bar's slider.
    fn scale(bar: &MotionBar) -> Option<gtk::Scale> {
        std::iter::successors(bar.root.first_child(), gtk::Widget::next_sibling)
            .find_map(|child| child.downcast::<gtk::Scale>().ok())
    }

    #[test]
    fn the_motion_bar_hears_people_not_its_own_show() {
        on_gtk_thread(motion_bar);
    }

    /// Showing the bar (twice in one turn, the range changing) moves the
    /// slider without calling back; people's moves are heard after the
    /// update; dropping the bar frees it.
    fn motion_bar() {
        let (owner, heard) = recorder();
        let bar = MotionBar::new(&owner, std::convert::identity);
        bar.show(door(40.0));
        assert!(!bar.root.is_visible(), "show is queued");
        bar.show(door(80.0));
        settle();
        assert!(bar.root.is_visible());
        let Some(scale) = scale(&bar) else {
            panic!("the bar has a slider");
        };
        assert!((scale.value() - 80.0).abs() < 1e-9, "the last show wins");
        assert!(heard.borrow().is_empty(), "show never calls back");
        scale.set_value(30.0);
        assert!(heard.borrow().is_empty(), "a move is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![MotionBarOutput::Moved(30.0)]);
        bar.set_orientation(gtk::Orientation::Vertical);
        settle();
        assert_eq!(scale.orientation(), gtk::Orientation::Vertical);
        bar.show(None);
        settle();
        assert!(!bar.root.is_visible());
        let root = bar.root.downgrade();
        let input = bar.controller.sender().clone();
        drop(bar);
        settle();
        assert!(root.upgrade().is_none(), "the bar is freed");
        assert!(input.send(MotionBarInput::Show(None)).is_err());
    }
}
