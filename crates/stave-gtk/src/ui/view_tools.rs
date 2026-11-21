//! The 3D view's floating controls (Main.dc.html, Exploded.dc.html): the
//! column of buttons at the top right (fit, front view, turn to the back,
//! reset the moved doors and drawers,
//! then X-ray and Pull apart, whose play button and "together … apart"
//! slider pop out to its left while it is on), the "Shared wood at first
//! contact" key shown while a problem is posed, and focus mode's "From
//! above" / "In 3D" switch. `view_layout` says where each one goes.
//!
//! Each control group is a relm4 component (docs/relm4-migration.md): its
//! model is the only state, only its `update` changes it, and its
//! widgets' handlers only send it messages. The structs named after them
//! ([`ViewToolbar`], [`CameraToolbar`], [`PlanSwitch`], [`EmptyView`])
//! own their controllers and keep the setters as plain methods.
//!
//! The window hears each group only through its outputs, forwarded to
//! its input when the group is made, which come from
//! people's presses alone: the Pull apart toggle and the plan switch's
//! buttons are set from the model with their handlers blocked
//! (`#[block_signal]`), so the window showing them is never heard back.

use adw::prelude::*;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt,
};
use stave_app::i18n::{fill_in, tr};

use super::icons::{self, PathIcon};
use super::slider::Slider;

/// A button holding one icon, with a tooltip and an accessible name.
fn icon_button(shape: icons::Shape, label: &str, classes: &[&str]) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&PathIcon::new(shape, 16))
        .tooltip_text(label)
        .css_classes(classes.to_vec())
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// An icon button that stays pressed while its tool is on.
fn icon_toggle(shape: icons::Shape, label: &str, tooltip: &str) -> gtk::ToggleButton {
    let button = gtk::ToggleButton::builder()
        .child(&PathIcon::new(shape, 16))
        .tooltip_text(tooltip)
        .css_classes(["stave-camera-button", "flat"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// The play button's tooltip.
fn play_tooltip(playing: bool) -> String {
    if playing {
        tr("Pause")
    } else {
        tr("Play: pull apart from together")
    }
}

/// The view's own tools: whether Pull apart is on, whether the tools are
/// offered, and the "together … apart" slider.
pub struct ViewToolbarComponent {
    /// Pull apart is on.
    on: bool,
    /// X-ray is on.
    xray: bool,
    /// The tools are offered (not while a problem is posed).
    offered: bool,
    /// Pulling apart is playing.
    playing: bool,
    /// How far apart, 0 to 1.
    slider: Slider,
    /// X-ray and Pull apart lie in a row or stand in a column, as the
    /// camera column they sit in does.
    orientation: gtk::Orientation,
}

/// What the view's tools are made with: their widgets, made by
/// [`ViewToolbar`] (tests press them).
#[derive(Debug)]
pub struct ViewToolbarInit {
    /// X-ray and Pull apart with the line above them: a root of the
    /// tools' own, which the camera column places at its foot.
    pub section: gtk::Box,
    /// The line above X-ray.
    pub line: gtk::Separator,
    /// See through everything but the selection.
    pub xray: gtk::ToggleButton,
    /// Pull apart, on or off.
    pub pull: gtk::ToggleButton,
    /// Plays the pulling apart from together to apart.
    pub play: gtk::Button,
    /// The play button's icon.
    pub play_icon: PathIcon,
}

/// What the view's tools are told.
#[derive(Debug)]
pub enum ViewToolbarInput {
    /// Show Pull apart on (with its slider at `amount`) or off, without
    /// telling anyone.
    Show {
        /// Pull apart is on.
        on: bool,
        /// How far apart, 0 to 1.
        amount: f32,
        /// Pulling apart is playing.
        playing: bool,
    },
    /// Offer the tools, or take them away while a problem is posed or a
    /// joint is in focus.
    Offer(bool),
    /// Show X-ray on or off, without telling anyone.
    SetXray(bool),
    /// Lay X-ray and Pull apart out in a row or a column, as the camera
    /// column they sit in is laid out.
    SetOrientation(gtk::Orientation),
    /// People turned X-ray on or off.
    XrayToggled(bool),
    /// People turned Pull apart on or off.
    PullToggled(bool),
    /// People pressed play (or pause).
    PlayPressed,
    /// People moved the slider.
    Moved(f64),
}

/// What the view's tools tell their owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewToolbarOutput {
    /// People turned X-ray on or off.
    Xray(bool),
    /// People turned Pull apart on or off.
    Pull(bool),
    /// People pressed play (or pause).
    Play,
    /// People moved the slider to this far apart, 0 to 1.
    Apart(f64),
    /// The slider popped out beside Pull apart (`true`) or went: the
    /// camera column meets it square while it shows.
    PoppedOut(bool),
}

#[relm4::component(pub)]
impl Component for ViewToolbarComponent {
    type Init = ViewToolbarInit;
    type Input = ViewToolbarInput;
    type Output = ViewToolbarOutput;
    type CommandOutput = ();

    view! {
        // The play button and the slider, floating beside Pull apart while
        // it is on.
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 4,
            add_css_class: "stave-camera",
            add_css_class: "stave-pull-popout",
            set_widget_name: "pull-apart-slider",
            update_property: &[gtk::accessible::Property::Label(&tr("Pull apart"))],
            #[watch]
            set_visible: model.on && model.offered,

            #[local_ref]
            play -> gtk::Button {
                #[watch]
                set_tooltip_text: Some(&play_tooltip(model.playing)),
                connect_clicked[sender] => move |_| {
                    sender.input_sender().emit(ViewToolbarInput::PlayPressed);
                },
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 2,
                set_hexpand: true,
                set_width_request: 200,
                add_css_class: "stave-tool-slider",

                #[local_ref]
                slider_area -> gtk::DrawingArea {},
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,

                    gtk::Label {
                        set_label: &tr("together"),
                        set_xalign: 0.0,
                        set_hexpand: true,
                        add_css_class: "stave-tool-end",
                    },
                    gtk::Label {
                        set_label: &tr("apart"),
                        set_xalign: 1.0,
                        add_css_class: "stave-tool-end",
                    },
                },
            },
        },

        #[local_ref]
        play_icon -> PathIcon {
            #[watch]
            set_shape: if model.playing { icons::PAUSE } else { icons::PLAY },
        },

        // X-ray and Pull apart, in the camera column.
        #[local_ref]
        section -> gtk::Box {
            #[watch]
            set_visible: model.offered,
            #[watch]
            set_orientation: model.orientation,
        },

        // The line between the camera buttons and the tools runs across
        // the column.
        #[local_ref]
        line -> gtk::Separator {
            #[watch]
            set_orientation: crosswise(model.orientation),
        },

        // Both toggles are set from the model without hearing it back.
        #[local_ref]
        xray -> gtk::ToggleButton {
            #[watch]
            #[block_signal(xray_toggled)]
            set_active: model.xray,
            connect_toggled[sender] => move |button| {
                sender
                    .input_sender()
                    .emit(ViewToolbarInput::XrayToggled(button.is_active()));
            } @xray_toggled,
        },

        #[local_ref]
        pull -> gtk::ToggleButton {
            #[watch]
            #[block_signal(pull_toggled)]
            set_active: model.on,
            connect_toggled[sender] => move |button| {
                sender
                    .input_sender()
                    .emit(ViewToolbarInput::PullToggled(button.is_active()));
            } @pull_toggled,
        },
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let ViewToolbarInit {
            section,
            line,
            xray,
            pull,
            play,
            play_icon,
        } = init;
        let (section, line, xray, pull, play, play_icon) =
            (&section, &line, &xray, &pull, &play, &play_icon);
        let slider = Slider::launch(
            &tr("How far apart"),
            24,
            12.0,
            1.5,
            sender.input_sender(),
            ViewToolbarInput::Moved,
        );
        let slider_area = &slider.root.clone();
        let model = ViewToolbarComponent {
            on: false,
            xray: false,
            offered: true,
            playing: false,
            slider,
            orientation: gtk::Orientation::Vertical,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: ViewToolbarInput, sender: ComponentSender<Self>, _: &Self::Root) {
        let popped = self.on && self.offered;
        let output = match message {
            ViewToolbarInput::Show {
                on,
                amount,
                playing,
            } => {
                self.on = on;
                self.playing = playing;
                self.slider.set_value(f64::from(amount));
                self.slider.set_value_text(&fill_in(
                    &tr("{percent}%, parts apart"),
                    &[("percent", &percent(amount))],
                ));
                None
            }
            ViewToolbarInput::Offer(offered) => {
                self.offered = offered;
                None
            }
            ViewToolbarInput::SetXray(on) => {
                self.xray = on;
                None
            }
            ViewToolbarInput::SetOrientation(orientation) => {
                self.orientation = orientation;
                None
            }
            ViewToolbarInput::XrayToggled(on) => {
                self.xray = on;
                Some(ViewToolbarOutput::Xray(on))
            }
            // The view follows people's toggle, so setting it back is a
            // change of nothing.
            ViewToolbarInput::PullToggled(on) => (self.on != on).then(|| {
                self.on = on;
                ViewToolbarOutput::Pull(on)
            }),
            ViewToolbarInput::PlayPressed => Some(ViewToolbarOutput::Play),
            ViewToolbarInput::Moved(value) => Some(ViewToolbarOutput::Apart(value)),
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake.
            let _ = sender.output(output);
        }
        let pops = self.on && self.offered;
        if pops != popped {
            let _ = sender.output(ViewToolbarOutput::PoppedOut(pops));
        }
    }
}

/// The view's own tools, in the camera column: X-ray and Pull apart, and
/// while Pull apart is on its play button and "together … apart" slider,
/// popping out to the left of its button. It owns the component:
/// dropping it shuts the component down. Setters are queued; the window
/// hears people's presses as [`ViewToolbarOutput`]s.
pub struct ViewToolbar {
    /// See through everything but the selection.
    #[cfg(test)]
    xray: gtk::ToggleButton,
    /// Pull apart, on or off.
    #[cfg(test)]
    pull: gtk::ToggleButton,
    /// The play button and the slider, floating beside Pull apart while
    /// it is on: a root for the owner to place.
    pub popout: gtk::Box,
    /// Plays the pulling apart from together to apart.
    #[cfg(test)]
    play: gtk::Button,
    /// X-ray and Pull apart with the line above them: a root of the
    /// tools' own, for the camera column to place at its foot.
    pub section: gtk::Box,
    /// The line above X-ray, turned crosswise to the column.
    #[cfg(test)]
    line: gtk::Separator,
    controller: Controller<ViewToolbarComponent>,
}

impl ViewToolbar {
    /// The tools, off; people's presses and slides go to `owner` as
    /// `message(output)`.
    pub fn new<M: 'static>(
        owner: &relm4::Sender<M>,
        message: impl Fn(ViewToolbarOutput) -> M + 'static,
    ) -> ViewToolbar {
        let xray = icon_toggle(
            icons::XRAY,
            &tr("X-ray"),
            &tr("X-ray: see through everything but the selection"),
        );
        let pull = icon_toggle(
            icons::PULL_APART,
            &tr("Pull apart"),
            &tr("Pull apart: move the parts away from each other along their joints"),
        );
        let play_icon = PathIcon::new(icons::PLAY, 13);
        let play = gtk::Button::builder()
            .child(&play_icon)
            .tooltip_text(play_tooltip(false))
            .css_classes(["stave-tool-play", "circular", "flat"])
            .valign(gtk::Align::Center)
            .build();
        play.update_property(&[gtk::accessible::Property::Label(&tr("Play pull apart"))]);
        let line = gtk::Separator::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-camera-separator"])
            .build();
        let section = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .build();
        section.append(&line);
        section.append(&xray);
        section.append(&pull);
        let controller = ViewToolbarComponent::builder()
            .launch(ViewToolbarInit {
                section: section.clone(),
                line: line.clone(),
                xray: xray.clone(),
                pull: pull.clone(),
                play: play.clone(),
                play_icon,
            })
            .forward(owner, message);
        ViewToolbar {
            #[cfg(test)]
            xray,
            #[cfg(test)]
            pull,
            popout: controller.widget().clone(),
            #[cfg(test)]
            play,
            section,
            #[cfg(test)]
            line,
            controller,
        }
    }

    /// Queues `input`; once the tools have shut down there is nobody to
    /// tell.
    fn send(&self, input: ViewToolbarInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Shows Pull apart on (with its slider at `amount`) or off, without
    /// calling back.
    pub fn show(&self, on: bool, amount: f32, playing: bool) {
        self.send(ViewToolbarInput::Show {
            on,
            amount,
            playing,
        });
    }

    /// Shows X-ray on or off, without calling back: the window turns it
    /// on for `--xray` and sets its own X-ray itself.
    pub fn show_xray(&self, on: bool) {
        self.send(ViewToolbarInput::SetXray(on));
    }

    /// Offers the tools, or takes them away while a problem is posed or
    /// a joint is in focus.
    pub fn offer(&self, offered: bool) {
        self.send(ViewToolbarInput::Offer(offered));
    }

    /// Lays X-ray and Pull apart out in a row (the camera column docked in
    /// a corner) or a column (docked on the right edge).
    pub fn set_orientation(&self, orientation: gtk::Orientation) {
        self.send(ViewToolbarInput::SetOrientation(orientation));
    }
}

/// An explode amount as a whole percentage.
pub fn percent(amount: f32) -> i32 {
    let percent = (amount.clamp(0.0, 1.0) * 100.0).round();
    // In 0..=100, so the conversion is exact.
    crate::convert::int_of_f32(percent)
}

/// The line between the camera buttons and the view's tools runs across
/// the column: crosswise to it.
fn crosswise(orientation: gtk::Orientation) -> gtk::Orientation {
    match orientation {
        gtk::Orientation::Vertical => gtk::Orientation::Horizontal,
        _ => gtk::Orientation::Vertical,
    }
}

/// The camera column: which way it runs, and whether Pull apart's slider
/// pops out beside it.
#[derive(Debug)]
pub struct CameraComponent {
    /// A column (docked on the right edge) or a row (docked in a corner).
    orientation: gtk::Orientation,
    /// Pull apart's slider pops out beside the column.
    joined: bool,
}

/// What the camera column is made with: its buttons, and the view's tools
/// to place at its foot.
#[derive(Debug)]
pub struct CameraInit {
    /// Frames everything (or the selection).
    pub fit: gtk::Button,
    /// Looks at the front.
    pub front: gtk::Button,
    /// Turns the view half way round.
    pub turn: gtk::Button,
    /// Puts every door and drawer moved with the motion bar back.
    pub reset: gtk::Button,
    /// X-ray and Pull apart, the view's tools' own root: placed at the
    /// foot, and otherwise left as the tools have it.
    pub section: gtk::Widget,
}

/// What the camera column is told.
#[derive(Debug)]
pub enum CameraInput {
    /// Lay the buttons out in a column (docked on the right edge) or a row
    /// (docked in a corner).
    SetOrientation(gtk::Orientation),
    /// Pull apart's slider popped out (or went): the column meets it
    /// square.
    Joined(bool),
    /// People pressed Front view.
    FrontPressed,
    /// People pressed Turn to the back.
    TurnPressed,
    /// People pressed Reset.
    ResetPressed,
}

/// What the camera column tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraOutput {
    /// People asked to look at the front.
    Front,
    /// People asked to turn the view half way round.
    Turn,
    /// People asked to put every moved door and drawer back.
    Reset,
}

#[relm4::component(pub)]
impl Component for CameraComponent {
    type Init = CameraInit;
    type Input = CameraInput;
    type Output = CameraOutput;
    type CommandOutput = ();

    view! {
        #[root]
        gtk::Box {
            set_spacing: 4,
            add_css_class: "stave-camera",
            set_widget_name: "camera-toolbar",
            #[watch]
            set_orientation: model.orientation,
            // A row is flat; while Pull apart's slider pops out beside the
            // column, the column's corner meets it square.
            #[watch]
            set_class_active: ("flat-row", model.orientation != gtk::Orientation::Vertical),
            #[watch]
            set_class_active: ("joined", model.joined),

            #[local_ref]
            fit -> gtk::Button {},
            #[local_ref]
            front -> gtk::Button {
                connect_clicked[sender] => move |_| {
                    sender.input_sender().emit(CameraInput::FrontPressed);
                },
            },
            #[local_ref]
            turn -> gtk::Button {
                connect_clicked[sender] => move |_| {
                    sender.input_sender().emit(CameraInput::TurnPressed);
                },
            },
            #[local_ref]
            reset -> gtk::Button {
                connect_clicked[sender] => move |_| {
                    sender.input_sender().emit(CameraInput::ResetPressed);
                },
            },
            #[local_ref]
            section -> gtk::Widget {},
        },
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let CameraInit {
            fit,
            front,
            turn,
            reset,
            section,
        } = init;
        // The slider starts hidden (Pull apart starts off).
        let model = CameraComponent {
            orientation: gtk::Orientation::Vertical,
            joined: false,
        };
        let (fit, front, turn, reset, section) = (&fit, &front, &turn, &reset, &section);
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: CameraInput, sender: ComponentSender<Self>, _: &Self::Root) {
        match message {
            CameraInput::SetOrientation(orientation) => self.orientation = orientation,
            CameraInput::Joined(joined) => self.joined = joined,
            CameraInput::FrontPressed => {
                let _ = sender.output(CameraOutput::Front);
            }
            CameraInput::TurnPressed => {
                let _ = sender.output(CameraOutput::Turn);
            }
            CameraInput::ResetPressed => {
                let _ = sender.output(CameraOutput::Reset);
            }
        }
    }
}

/// The column of buttons at the top right of the view: fit, front view,
/// turn to the back and reset, then X-ray and Pull apart. It owns the
/// component. Setters are queued; the window hears people's presses as
/// [`CameraOutput`]s.
pub struct CameraToolbar {
    /// The column, to float on the view.
    pub root: gtk::Box,
    /// Looks at the front.
    #[cfg(test)]
    front: gtk::Button,
    /// Turns the view half way round.
    #[cfg(test)]
    turn: gtk::Button,
    /// Puts the moved doors and drawers back.
    #[cfg(test)]
    reset: gtk::Button,
    controller: Controller<CameraComponent>,
}

impl CameraToolbar {
    /// The buttons, with the view's own tools at the foot; people's
    /// presses go to `owner` as `message(output)`.
    pub fn new<M: 'static>(
        tools: &ViewToolbar,
        owner: &relm4::Sender<M>,
        message: impl Fn(CameraOutput) -> M + 'static,
    ) -> CameraToolbar {
        let fit = icon_button(
            icons::FIT,
            &tr("Fit to view (F)"),
            &["stave-camera-button", "flat"],
        );
        // Frames everything (or the selection) through the window's
        // `frame` action, as F does.
        fit.set_action_name(Some("win.frame"));
        let front = icon_button(
            icons::FRONT,
            &tr("Front view"),
            &["stave-camera-button", "flat"],
        );
        let turn = icon_button(
            icons::TURN,
            &tr("Turn to the back"),
            &["stave-camera-button", "flat"],
        );
        let reset = icon_button(
            icons::RESET,
            &tr("Reset: close every door and drawer you moved"),
            &["stave-camera-button", "flat"],
        );
        let controller = CameraComponent::builder()
            .launch(CameraInit {
                fit: fit.clone(),
                front: front.clone(),
                turn: turn.clone(),
                reset: reset.clone(),
                section: tools.section.clone().upcast(),
            })
            .forward(owner, message);
        CameraToolbar {
            root: controller.widget().clone(),
            #[cfg(test)]
            front,
            #[cfg(test)]
            turn,
            #[cfg(test)]
            reset,
            controller,
        }
    }

    /// Lays the buttons out in a column (docked on the right edge) or a
    /// row (docked in a corner); the view's tools are told the same by
    /// the window ([`ViewToolbar::set_orientation`]).
    pub fn set_orientation(&self, orientation: gtk::Orientation) {
        self.send(CameraInput::SetOrientation(orientation));
    }

    /// Meets Pull apart's slider square while it pops out beside the
    /// column (the view's tools say when, [`ViewToolbarOutput::PoppedOut`]).
    pub fn set_joined(&self, joined: bool) {
        self.send(CameraInput::Joined(joined));
    }

    /// Queues `input`; once the column has shut down there is nobody to
    /// tell.
    fn send(&self, input: CameraInput) {
        let _ = self.controller.sender().send(input);
    }
}

/// The key shown while a problem is posed: a red square and "Shared wood
/// at first contact". It floats on the 3D view, placed exactly by the
/// floats.
pub fn contact_key() -> gtk::Box {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .css_classes(["stave-contact-key"])
        .visible(false)
        .build();
    root.append(&icons::swatch(
        gtk::gdk::RGBA::new(0.878, 0.106, 0.141, 1.0),
        false,
        10,
    ));
    root.append(&gtk::Label::new(Some(&tr("Shared wood at first contact"))));
    root
}

/// Focus mode's switch: which view is on.
#[derive(Debug, Default)]
pub struct PlanSwitchComponent {
    /// The view from above is on (else the view in 3D).
    plan: bool,
}

/// What the switch is made with: its two buttons, grouped.
#[derive(Debug)]
pub struct PlanSwitchInit {
    /// From above (plan).
    pub from_above: gtk::ToggleButton,
    /// In 3D.
    pub in_3d: gtk::ToggleButton,
}

/// What the switch is told.
#[derive(Debug)]
pub enum PlanSwitchInput {
    /// Show which view is on, without telling anyone.
    Show {
        /// The view from above is on.
        plan: bool,
    },
    /// "From above" turned on or off, by people (pressing either button).
    Toggled(bool),
}

/// What the switch tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanSwitchOutput {
    /// People chose the view from above (`true`) or in 3D.
    FromAbove(bool),
}

#[relm4::component(pub)]
impl Component for PlanSwitchComponent {
    type Init = PlanSwitchInit;
    type Input = PlanSwitchInput;
    type Output = PlanSwitchOutput;
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 2,
            // It floats on the 3D view, placed exactly by the floats.
            add_css_class: "stave-plan-switch",
            set_visible: false,
            update_property: &[gtk::accessible::Property::Label(&tr("Hinge view"))],

            // Both buttons are set from the model in `Show`, with this
            // handler blocked (pressing either button toggles this one).
            #[local_ref]
            from_above -> gtk::ToggleButton {
                connect_toggled[sender] => move |button| {
                    sender
                        .input_sender()
                        .emit(PlanSwitchInput::Toggled(button.is_active()));
                } @from_above_toggled,
            },
            #[local_ref]
            in_3d -> gtk::ToggleButton {
                // It opens in 3D. "From above" was off already, so its
                // handler does not run.
                set_active: !model.plan,
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let PlanSwitchInit { from_above, in_3d } = init;
        let (from_above, in_3d) = (&from_above, &in_3d);
        let model = PlanSwitchComponent::default();
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: PlanSwitchInput,
        sender: ComponentSender<Self>,
        _: &Self::Root,
    ) {
        match message {
            PlanSwitchInput::Show { plan } => {
                self.plan = plan;
                // What `#[block_signal]` does, round both buttons: turning
                // either on toggles "From above".
                widgets.from_above.block_signal(&widgets.from_above_toggled);
                if plan {
                    widgets.from_above.set_active(true);
                } else {
                    widgets.in_3d.set_active(true);
                }
                widgets
                    .from_above
                    .unblock_signal(&widgets.from_above_toggled);
            }
            PlanSwitchInput::Toggled(plan) => {
                if self.plan != plan {
                    self.plan = plan;
                    let _ = sender.output(PlanSwitchOutput::FromAbove(plan));
                }
            }
        }
    }
}

/// Focus mode's switch between the view from above and the view in 3D. It
/// owns the component. [`PlanSwitch::show`] is queued and never calls
/// back; the window hears people's presses as [`PlanSwitchOutput`]s.
pub struct PlanSwitch {
    /// The switch, to overlay on the view.
    pub root: gtk::Box,
    /// From above (plan).
    #[cfg(test)]
    from_above: gtk::ToggleButton,
    /// In 3D.
    #[cfg(test)]
    in_3d: gtk::ToggleButton,
    controller: Controller<PlanSwitchComponent>,
}

impl PlanSwitch {
    /// The switch, in 3D; people's choices go to `owner` as
    /// `message(output)`.
    pub fn new<M: 'static>(
        owner: &relm4::Sender<M>,
        message: impl Fn(PlanSwitchOutput) -> M + 'static,
    ) -> PlanSwitch {
        let toggle = |shape: icons::Shape, label: &str| {
            let button = gtk::ToggleButton::builder()
                .child(&PathIcon::new(shape, 17))
                .tooltip_text(label)
                .css_classes(["stave-plan-button"])
                .build();
            button.update_property(&[gtk::accessible::Property::Label(label)]);
            button
        };
        let from_above = toggle(icons::FROM_ABOVE, &tr("From above"));
        let in_3d = toggle(icons::PARTS, &tr("In 3D"));
        in_3d.set_group(Some(&from_above));
        let controller = PlanSwitchComponent::builder()
            .launch(PlanSwitchInit {
                from_above: from_above.clone(),
                in_3d: in_3d.clone(),
            })
            .forward(owner, message);
        PlanSwitch {
            root: controller.widget().clone(),
            #[cfg(test)]
            from_above,
            #[cfg(test)]
            in_3d,
            controller,
        }
    }

    /// Shows which view is on, without calling back.
    pub fn show(&self, plan: bool) {
        // Once the switch has shut down there is nobody to tell.
        let _ = self
            .controller
            .sender()
            .send(PlanSwitchInput::Show { plan });
    }
}

/// What the 3D view's area says when there is nothing to draw.
#[derive(Debug)]
pub struct EmptyViewComponent {
    /// "Checking…", or why there is nothing to draw.
    title: String,
    /// What it means; empty for nothing more.
    description: String,
    /// "Open a Project…" helps here.
    offers_open: bool,
    /// The page takes the dark stage colour.
    dark: bool,
}

/// What the empty view is told.
#[derive(Debug)]
pub enum EmptyViewInput {
    /// Say why there is nothing to draw.
    Show {
        /// The title.
        title: String,
        /// What it means; empty for nothing more.
        description: String,
        /// Offer "Open a Project…".
        offers_open: bool,
    },
    /// Take the dark stage colour (`true`) or the light one.
    Dark(bool),
}

#[relm4::component(pub)]
impl Component for EmptyViewComponent {
    type Init = gtk::Button;
    type Input = EmptyViewInput;
    type Output = ();
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            add_css_class: "stave-stage",
            add_css_class: "stave-empty",
            #[watch]
            set_class_active: ("dark", model.dark),

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 12,
                set_valign: gtk::Align::Center,
                set_halign: gtk::Align::Center,
                set_vexpand: true,
                set_margin_start: 24,
                set_margin_end: 24,

                PathIcon::new(icons::PARTS, 48) {
                    add_css_class: "stave-empty-icon",
                },
                gtk::Label {
                    set_wrap: true,
                    set_justify: gtk::Justification::Center,
                    add_css_class: "stave-empty-title",
                    #[watch]
                    set_label: &model.title,
                },
                gtk::Label {
                    set_wrap: true,
                    set_justify: gtk::Justification::Center,
                    set_max_width_chars: 48,
                    add_css_class: "stave-empty-description",
                    #[watch]
                    set_label: &model.description,
                    #[watch]
                    set_visible: !model.description.is_empty(),
                },
                #[local_ref]
                open -> gtk::Button {
                    #[watch]
                    set_visible: model.offers_open,
                },
            },
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let open = &init;
        let model = EmptyViewComponent {
            title: tr("Checking…"),
            description: String::new(),
            offers_open: false,
            dark: false,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: EmptyViewInput, _: ComponentSender<Self>, _: &Self::Root) {
        match message {
            EmptyViewInput::Show {
                title,
                description,
                offers_open,
            } => {
                self.title = title;
                self.description = description;
                self.offers_open = offers_open;
            }
            EmptyViewInput::Dark(dark) => self.dark = dark,
        }
    }
}

/// What the 3D view's area shows when there is nothing to draw (still
/// checking, a library folder, a design that doesn't solve): an icon, a
/// title, what it means, and "Open a Project…" where that helps. Never a
/// blank area. It keeps the API the window still uses and owns the
/// component. Setters are queued.
pub struct EmptyView {
    /// The page, on the stage colour.
    pub root: gtk::Box,
    /// Opens another project folder (`win.open`).
    #[cfg(test)]
    pub open: gtk::Button,
    controller: Controller<EmptyViewComponent>,
}

impl EmptyView {
    /// The page, saying "Checking…".
    pub fn new() -> EmptyView {
        let open = gtk::Button::builder()
            .label(tr("Open a Project…"))
            .action_name("win.open")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        // It tells nobody anything: its button uses the window's action.
        let controller = EmptyViewComponent::builder().launch(open.clone()).detach();
        EmptyView {
            root: controller.widget().clone(),
            #[cfg(test)]
            open,
            controller,
        }
    }

    /// Says why there is nothing to draw.
    pub fn show(&self, title: &str, description: &str, offers_open: bool) {
        self.send(EmptyViewInput::Show {
            title: title.to_owned(),
            description: description.to_owned(),
            offers_open,
        });
    }

    /// Takes the dark stage colour, or the light one, as the 3D view's
    /// stage does.
    pub fn show_dark(&self, dark: bool) {
        self.send(EmptyViewInput::Dark(dark));
    }

    /// Queues `input`; once the page has shut down there is nobody to
    /// tell.
    fn send(&self, input: EmptyViewInput) {
        let _ = self.controller.sender().send(input);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{handle_each, on_gtk_thread, recorder, settle};

    /// What people did, of the outputs heard: without the slider popping
    /// out or going, which the tools publish whoever turned Pull apart on.
    fn people(heard: &[ViewToolbarOutput]) -> Vec<ViewToolbarOutput> {
        heard
            .iter()
            .copied()
            .filter(|output| !matches!(output, ViewToolbarOutput::PoppedOut(_)))
            .collect()
    }

    /// View tools whose outputs nobody hears.
    fn view_tools() -> ViewToolbar {
        let (owner, _) = recorder();
        ViewToolbar::new(&owner, std::convert::identity)
    }

    #[test]
    fn a_dropped_view_toolbar_is_freed() {
        on_gtk_thread(view_toolbar_is_freed);
    }

    /// Dropping the toolbar shuts its component down: its widgets, its
    /// slider and the window's message maker are freed.
    fn view_toolbar_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let (owner, _) = recorder();
        let tools = ViewToolbar::new(&owner, move |output| {
            let _ = &held;
            output
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let popout = tools.popout.downgrade();
        let pull = tools.pull.downgrade();
        let slider = tools.controller.model().slider.root.downgrade();
        let input = tools.controller.sender().clone();
        settle();
        assert!(popout.upgrade().is_some(), "the tools live while held");
        drop(tools);
        settle();
        assert!(popout.upgrade().is_none(), "the pop-out is freed");
        assert!(pull.upgrade().is_none(), "Pull apart is freed");
        assert!(slider.upgrade().is_none(), "the slider is freed");
        assert!(input.send(ViewToolbarInput::Offer(true)).is_err());
        assert!(callback.upgrade().is_none(), "the message maker is freed");
    }

    #[test]
    fn showing_pull_apart_does_not_read_as_people_toggling_it() {
        on_gtk_thread(show_does_not_call_back);
    }

    /// `show` sets the toggle from the model with its handler blocked:
    /// `show` and `offer` never send an output of people's (only that the
    /// slider popped out and went).
    fn show_does_not_call_back() {
        let (owner, heard) = recorder();
        let tools = ViewToolbar::new(&owner, std::convert::identity);
        tools.show(true, 0.4, true);
        assert!(!tools.pull.is_active(), "show is queued");
        settle();
        assert!(tools.pull.is_active());
        assert!(tools.popout.is_visible(), "the slider pops out");
        assert_eq!(tools.controller.model().slider.value(), f64::from(0.4_f32));
        assert_eq!(tools.play.tooltip_text().as_deref(), Some("Pause"));
        tools.offer(false);
        settle();
        assert!(
            !tools.popout.is_visible(),
            "not offered while a problem is posed"
        );
        assert!(!tools.section.is_visible());
        tools.show(false, 0.0, false);
        tools.offer(true);
        settle();
        assert!(!tools.pull.is_active());
        assert!(!tools.popout.is_visible());
        assert!(
            people(&heard.borrow()).is_empty(),
            "setters never call back"
        );
        assert_eq!(
            *heard.borrow(),
            vec![
                ViewToolbarOutput::PoppedOut(true),
                ViewToolbarOutput::PoppedOut(false),
            ]
        );
    }

    #[test]
    fn showing_pull_apart_several_times_in_one_turn_never_calls_back() {
        on_gtk_thread(several_shows_never_call_back);
    }

    /// The window can show Pull apart several times in one turn of the
    /// main loop (leaving a problem and opening another). Each `Show` is
    /// applied in order with the toggle's handler blocked, so none reads
    /// as people pressing it and the last one wins.
    fn several_shows_never_call_back() {
        let (owner, heard) = recorder();
        let tools = ViewToolbar::new(&owner, std::convert::identity);
        tools.show(true, 0.0, false);
        tools.show(false, 0.0, false);
        tools.show(true, 0.0, false);
        settle();
        assert!(tools.pull.is_active());
        assert!(tools.controller.model().on, "the model agrees");
        tools.show(false, 0.0, false);
        settle();
        assert!(!tools.pull.is_active());
        assert!(
            people(&heard.borrow()).is_empty(),
            "the component never called back"
        );
    }

    #[test]
    fn showing_xray_sets_the_toggle_without_calling_back() {
        on_gtk_thread(showing_xray_never_calls_back);
    }

    /// `--xray` turns X-ray on through the toolbar's own input: the toggle
    /// shows it, the model agrees, and nobody hears it back as people's
    /// press. People's press afterwards is heard as before.
    fn showing_xray_never_calls_back() {
        let (owner, heard) = recorder();
        let tools = ViewToolbar::new(&owner, std::convert::identity);
        tools.show_xray(true);
        assert!(!tools.xray.is_active(), "only queued so far");
        settle();
        assert!(tools.xray.is_active());
        assert!(tools.controller.model().xray, "the model agrees");
        assert!(heard.borrow().is_empty(), "the window's show is not heard");
        tools.xray.set_active(false);
        settle();
        assert_eq!(*heard.borrow(), vec![ViewToolbarOutput::Xray(false)]);
        assert!(!tools.controller.model().xray);
    }

    #[test]
    fn a_toggle_fired_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(toggles_are_queued);
    }

    /// People pressing Pull apart only queues a message; the output comes
    /// after the update. The window showing it off while it handles that
    /// output is queued too, and is not heard back as people's input.
    fn toggles_are_queued() {
        let (owner, outputs) = relm4::channel();
        let tools = Rc::new(ViewToolbar::new(&owner, std::convert::identity));
        let heard: Rc<RefCell<Vec<ViewToolbarOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&tools);
        handle_each(outputs, move |output| {
            let Some(tools) = weak.upgrade() else {
                return;
            };
            // Reading the model here would panic if the toolbar were
            // still in its update.
            let on = tools.controller.model().on;
            log.borrow_mut().push(output);
            if output == ViewToolbarOutput::Pull(true) {
                assert!(on, "the toolbar follows people's toggle");
                tools.show(false, 0.0, false);
            }
        });
        tools.pull.set_active(true);
        tools.xray.set_active(true);
        tools.play.emit_clicked();
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        settle();
        assert_eq!(
            people(&heard.borrow()),
            vec![
                ViewToolbarOutput::Pull(true),
                ViewToolbarOutput::Xray(true),
                ViewToolbarOutput::Play
            ],
            "the window's show is not heard as Pull(false)"
        );
        assert!(!tools.pull.is_active(), "the window's show came last");
    }

    #[test]
    fn the_camera_column_turns_with_its_tools_and_meets_the_pop_out() {
        on_gtk_thread(camera_column);
    }

    fn camera_column() {
        let tools = view_tools();
        let (owner, _) = recorder();
        let camera = CameraToolbar::new(&tools, &owner, std::convert::identity);
        camera.set_orientation(gtk::Orientation::Horizontal);
        assert_eq!(
            camera.root.orientation(),
            gtk::Orientation::Vertical,
            "queued"
        );
        settle();
        assert_eq!(camera.root.orientation(), gtk::Orientation::Horizontal);
        assert!(camera.root.has_css_class("flat-row"));
        assert!(
            tools.section.is_ancestor(&camera.root),
            "the tools sit at its foot"
        );
        assert_eq!(
            tools.section.orientation(),
            gtk::Orientation::Vertical,
            "the tools are laid out by their own component"
        );
        camera.set_joined(true);
        settle();
        assert!(
            camera.root.has_css_class("joined"),
            "the pop-out is beside it"
        );
        camera.set_joined(false);
        settle();
        assert!(!camera.root.has_css_class("joined"));
    }

    #[test]
    fn the_view_tools_turn_and_say_when_the_slider_pops_out() {
        on_gtk_thread(view_tools_turn_and_pop_out);
    }

    /// The tools lay X-ray and Pull apart out as they are told, the line
    /// above them crosswise; they say when the slider pops out (Pull apart
    /// on and the tools offered) and when it goes, once each time.
    fn view_tools_turn_and_pop_out() {
        let (owner, heard) = recorder();
        let tools = ViewToolbar::new(&owner, std::convert::identity);
        tools.set_orientation(gtk::Orientation::Horizontal);
        settle();
        assert_eq!(tools.section.orientation(), gtk::Orientation::Horizontal);
        assert_eq!(tools.line.orientation(), gtk::Orientation::Vertical);
        tools.show(true, 0.0, false);
        tools.show(true, 0.5, false);
        settle();
        tools.offer(false);
        settle();
        tools.show(false, 0.0, false);
        tools.offer(true);
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                ViewToolbarOutput::PoppedOut(true),
                ViewToolbarOutput::PoppedOut(false),
            ],
            "out, then away when the tools are taken away; offered again \
             with Pull apart off, it stays away"
        );
    }

    #[test]
    fn camera_presses_are_heard_after_the_update_and_the_column_is_freed() {
        on_gtk_thread(camera_presses);
    }

    fn camera_presses() {
        let tools = view_tools();
        let (owner, heard) = recorder();
        let camera = CameraToolbar::new(&tools, &owner, std::convert::identity);
        camera.front.emit_clicked();
        camera.turn.emit_clicked();
        camera.reset.emit_clicked();
        assert!(heard.borrow().is_empty(), "queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![CameraOutput::Front, CameraOutput::Turn, CameraOutput::Reset]
        );
        let root = camera.root.downgrade();
        let input = camera.controller.sender().clone();
        drop(camera);
        settle();
        assert!(root.upgrade().is_none(), "the column is freed");
        assert!(input.send(CameraInput::Joined(false)).is_err());
        drop(tools);
    }

    #[test]
    fn the_plan_switch_hears_people_not_itself() {
        on_gtk_thread(plan_switch);
    }

    /// Showing the view from above sets the buttons with the handler
    /// blocked: only people's presses are heard, each after its update.
    fn plan_switch() {
        let (owner, outputs) = relm4::channel();
        let switch = Rc::new(PlanSwitch::new(&owner, std::convert::identity));
        settle();
        assert!(switch.in_3d.is_active(), "it opens in 3D");
        let heard: Rc<RefCell<Vec<PlanSwitchOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&switch);
        handle_each(outputs, move |output| {
            log.borrow_mut().push(output);
            if let Some(switch) = weak.upgrade() {
                // The window shows it back while it handles the output.
                switch.show(true);
            }
        });
        switch.show(true);
        settle();
        assert!(switch.from_above.is_active());
        switch.show(false);
        settle();
        assert!(switch.in_3d.is_active());
        assert!(heard.borrow().is_empty(), "show never calls back");
        switch.from_above.set_active(true);
        assert!(heard.borrow().is_empty(), "a press is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![PlanSwitchOutput::FromAbove(true)]);
        switch.in_3d.set_active(true);
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                PlanSwitchOutput::FromAbove(true),
                PlanSwitchOutput::FromAbove(false)
            ]
        );
        assert!(switch.from_above.is_active(), "the window's show came last");
        let root = switch.root.downgrade();
        drop(switch);
        settle();
        assert!(root.upgrade().is_none(), "the switch is freed");
    }

    #[test]
    fn showing_the_plan_several_times_in_one_turn_never_calls_back() {
        on_gtk_thread(several_plan_shows_never_call_back);
    }

    /// Leaving focus on one joint and entering another shows the switch
    /// twice in one turn. Each `Show` is applied in order with the
    /// handler blocked, so the first never writes a stale plan back and
    /// the last one wins.
    fn several_plan_shows_never_call_back() {
        let (owner, heard) = recorder();
        let switch = PlanSwitch::new(&owner, std::convert::identity);
        settle();
        switch.show(true);
        assert!(!switch.from_above.is_active(), "show is queued");
        switch.show(false);
        switch.show(true);
        settle();
        assert!(switch.from_above.is_active());
        assert!(switch.controller.model().plan, "the model agrees");
        switch.show(false);
        settle();
        assert!(switch.in_3d.is_active());
        assert!(!switch.controller.model().plan);
        assert!(heard.borrow().is_empty(), "the component never called back");
    }

    #[test]
    fn the_empty_view_says_why_and_is_freed() {
        on_gtk_thread(empty_view);
    }

    fn empty_view() {
        let empty = EmptyView::new();
        settle();
        assert_eq!(empty.controller.model().title, "Checking…");
        assert!(!empty.open.is_visible());
        empty.show("A library folder", "Open a project to see it.", true);
        settle();
        assert_eq!(empty.controller.model().title, "A library folder");
        assert!(empty.open.is_visible());
        empty.show_dark(true);
        assert!(!empty.root.has_css_class("dark"), "queued");
        settle();
        assert!(empty.root.has_css_class("dark"), "it styles itself");
        empty.show_dark(false);
        settle();
        assert!(!empty.root.has_css_class("dark"));
        let root = empty.root.downgrade();
        let input = empty.controller.sender().clone();
        drop(empty);
        settle();
        assert!(root.upgrade().is_none(), "the page is freed");
        assert!(
            input
                .send(EmptyViewInput::Show {
                    title: String::new(),
                    description: String::new(),
                    offers_open: false,
                })
                .is_err()
        );
    }
}
