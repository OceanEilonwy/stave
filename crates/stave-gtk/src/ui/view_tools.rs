//! The 3D view's floating controls (Main.dc.html, Exploded.dc.html): the
//! column of buttons at the top right (fit, front view, turn to the back,
//! then X-ray and Pull apart, whose play button and "together … apart"
//! slider pop out to its left while it is on), the "Shared wood at first
//! contact" key shown while a problem is posed, and focus mode's "From
//! above" / "In 3D" switch. `view_layout` says where each one goes.
//!
//! Each control group is a relm4 component (docs/relm4-migration.md): its
//! model is the only state, only its `update` changes it, and its
//! widgets' handlers only send it messages. Programmatic changes that
//! would fire a toggle's handler are made with the handler blocked
//! (`#[block_signal]`), so they never read as people's input. The structs
//! named after them ([`ViewToolbar`], [`CameraToolbar`], [`PlanSwitch`],
//! [`EmptyView`]) keep the older API in front of the components until the
//! window moves to relm4 too.

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt};

use super::focus_stage::SliderChanges;
use super::icons::{self, PathIcon};
use super::relm4_shim::Bridge;
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
fn play_tooltip(playing: bool) -> &'static str {
    if playing {
        "Pause"
    } else {
        "Play: pull apart from together"
    }
}

/// The view's own tools: whether Pull apart is on, whether the tools are
/// offered, and the "together … apart" slider.
pub struct ViewToolbarComponent {
    /// Pull apart is on.
    on: bool,
    /// The tools are offered (not while a problem is posed).
    offered: bool,
    /// Pulling apart is playing.
    playing: bool,
    /// How far apart, 0 to 1.
    slider: Slider,
}

/// What the view's tools are made with: the widgets the window and the
/// camera column still reach.
#[derive(Debug)]
pub struct ViewToolbarInit {
    /// X-ray and Pull apart with the line above them, for the camera
    /// column.
    pub section: gtk::Box,
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
            update_property: &[gtk::accessible::Property::Label("Pull apart")],
            #[watch]
            set_visible: model.on && model.offered,

            #[local_ref]
            play -> gtk::Button {
                #[watch]
                set_tooltip_text: Some(play_tooltip(model.playing)),
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
                        set_label: "together",
                        set_xalign: 0.0,
                        set_hexpand: true,
                        add_css_class: "stave-tool-end",
                    },
                    gtk::Label {
                        set_label: "apart",
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
        },

        #[local_ref]
        xray -> gtk::ToggleButton {
            connect_toggled[sender] => move |button| {
                sender
                    .input_sender()
                    .emit(ViewToolbarInput::XrayToggled(button.is_active()));
            },
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
            xray,
            pull,
            play,
            play_icon,
        } = init;
        let (section, xray, pull, play, play_icon) = (&section, &xray, &pull, &play, &play_icon);
        let slider = Slider::new("How far apart", 18, 12.0, 1.5);
        let input = sender.input_sender().clone();
        slider.connect_changed(move |value| input.emit(ViewToolbarInput::Moved(value)));
        let slider_area = &slider.root.clone();
        let model = ViewToolbarComponent {
            on: false,
            offered: true,
            playing: false,
            slider,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: ViewToolbarInput, sender: ComponentSender<Self>, _: &Self::Root) {
        let output = match message {
            ViewToolbarInput::Show {
                on,
                amount,
                playing,
            } => {
                self.on = on;
                self.playing = playing;
                self.slider.set_value(f64::from(amount));
                self.slider
                    .set_value_text(&format!("{}%, parts apart", percent(amount)));
                None
            }
            ViewToolbarInput::Offer(offered) => {
                self.offered = offered;
                None
            }
            ViewToolbarInput::XrayToggled(on) => Some(ViewToolbarOutput::Xray(on)),
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
    }
}

/// The view's own tools, in the camera column: X-ray and Pull apart, and
/// while Pull apart is on its play button and "together … apart" slider,
/// popping out to the left of its button. It keeps the API the window
/// still uses and owns the component: dropping it shuts the component
/// down. Setters are queued.
pub struct ViewToolbar {
    /// See through everything but the selection.
    pub xray: gtk::ToggleButton,
    /// Pull apart, on or off.
    pub pull: gtk::ToggleButton,
    /// The play button and the slider, floating beside Pull apart while
    /// it is on.
    pub popout: gtk::Box,
    /// Plays the pulling apart from together to apart.
    pub play: gtk::Button,
    /// How far apart, 0 to 1: the toolbar sets it in [`ViewToolbar::show`].
    pub slider: SliderChanges,
    /// X-ray and Pull apart with the line above them, for the camera
    /// column.
    section: gtk::Box,
    /// The line above X-ray, turned crosswise to the column.
    line: gtk::Separator,
    bridge: Bridge<ViewToolbarComponent>,
}

impl ViewToolbar {
    /// The tools, off.
    pub fn new() -> ViewToolbar {
        let xray = icon_toggle(
            icons::XRAY,
            "X-ray",
            "X-ray: see through everything but the selection",
        );
        let pull = icon_toggle(
            icons::PULL_APART,
            "Pull apart",
            "Pull apart: move the parts away from each other along their joints",
        );
        let play_icon = PathIcon::new(icons::PLAY, 13);
        let play = gtk::Button::builder()
            .child(&play_icon)
            .tooltip_text(play_tooltip(false))
            .css_classes(["stave-tool-play", "circular", "flat"])
            .valign(gtk::Align::Center)
            .build();
        play.update_property(&[gtk::accessible::Property::Label("Play pull apart")]);
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
        let bridge = Bridge::<ViewToolbarComponent>::launch(ViewToolbarInit {
            section: section.clone(),
            xray: xray.clone(),
            pull: pull.clone(),
            play: play.clone(),
            play_icon,
        });
        let (slider, changed) = SliderChanges::new();
        bridge.connect(move |output| match output {
            ViewToolbarOutput::Apart(value) => changed.deliver(value),
            // The window hears these on the buttons themselves.
            ViewToolbarOutput::Xray(_) | ViewToolbarOutput::Pull(_) | ViewToolbarOutput::Play => {}
        });
        ViewToolbar {
            xray,
            pull,
            popout: bridge.widget().clone(),
            play,
            slider,
            section,
            line,
            bridge,
        }
    }

    /// Shows Pull apart on (with its slider at `amount`) or off.
    pub fn show(&self, on: bool, amount: f32, playing: bool) {
        self.bridge.send(ViewToolbarInput::Show {
            on,
            amount,
            playing,
        });
    }

    /// Offers the tools, or takes them away while a problem is posed or
    /// a joint is in focus.
    pub fn offer(&self, offered: bool) {
        self.bridge.send(ViewToolbarInput::Offer(offered));
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
pub struct CameraComponent {
    /// A column (docked on the right edge) or a row (docked in a corner).
    orientation: gtk::Orientation,
    /// Pull apart's slider pops out beside the column.
    joined: bool,
    /// Where the column's messages go, for the window's orientation
    /// setter.
    input: relm4::Sender<CameraInput>,
    /// Pull apart's slider, which belongs to the view's tools, and the
    /// handler watching it come and go, let go when the column shuts
    /// down.
    popout: gtk::Box,
    joined_handler: Option<gtk::glib::SignalHandlerId>,
}

/// What the camera column is made with: its buttons and the view's tools
/// at its foot.
#[derive(Debug)]
pub struct CameraInit {
    /// Frames everything (or the selection).
    pub fit: gtk::Button,
    /// Looks at the front.
    pub front: gtk::Button,
    /// Turns the view half way round.
    pub turn: gtk::Button,
    /// X-ray and Pull apart, at the foot.
    pub section: gtk::Box,
    /// The line above them.
    pub line: gtk::Separator,
    /// Pull apart's slider, which pops out beside the column.
    pub popout: gtk::Box,
}

/// What the camera column is told.
#[derive(Debug)]
pub enum CameraInput {
    /// Lay the buttons out in a column (docked on the right edge) or a row
    /// (docked in a corner), X-ray and Pull apart with them.
    SetOrientation(gtk::Orientation),
    /// Pull apart's slider popped out (or went).
    Joined(bool),
    /// People pressed Front view.
    FrontPressed,
    /// People pressed Turn to the back.
    TurnPressed,
}

/// What the camera column tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraOutput {
    /// People asked to look at the front.
    Front,
    /// People asked to turn the view half way round.
    Turn,
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
            section -> gtk::Box {
                #[watch]
                set_orientation: model.orientation,
            },
        },

        #[local_ref]
        line -> gtk::Separator {
            #[watch]
            set_orientation: crosswise(model.orientation),
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
            section,
            line,
            popout,
        } = init;
        let input = sender.input_sender().clone();
        let joined = input.clone();
        let joined_handler = popout.connect_visible_notify(move |popout| {
            joined.emit(CameraInput::Joined(popout.is_visible()));
        });
        let model = CameraComponent {
            orientation: gtk::Orientation::Vertical,
            joined: popout.is_visible(),
            input,
            popout,
            joined_handler: Some(joined_handler),
        };
        let (fit, front, turn, section, line) = (&fit, &front, &turn, &section, &line);
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
        }
    }

    fn shutdown(&mut self, _: &mut Self::Widgets, _: relm4::Sender<CameraOutput>) {
        // The pop-out may outlive the column: it stops telling it.
        if let Some(handler) = self.joined_handler.take() {
            self.popout.disconnect(handler);
        }
    }
}

/// The column of buttons at the top right of the view: fit, front view
/// and turn to the back, then X-ray and Pull apart. It keeps the API the
/// window still uses and owns the component. Setters are queued.
pub struct CameraToolbar {
    /// The column, to float on the view.
    pub root: gtk::Box,
    /// Frames everything (or the selection).
    pub fit: gtk::Button,
    /// Looks at the front.
    pub front: gtk::Button,
    /// Turns the view half way round.
    pub turn: gtk::Button,
    bridge: Bridge<CameraComponent>,
}

impl CameraToolbar {
    /// The buttons, with the view's own tools at the foot.
    pub fn new(tools: &ViewToolbar) -> CameraToolbar {
        let fit = icon_button(
            icons::FIT,
            "Fit to view (F)",
            &["stave-camera-button", "flat"],
        );
        let front = icon_button(icons::FRONT, "Front view", &["stave-camera-button", "flat"]);
        let turn = icon_button(
            icons::TURN,
            "Turn to the back",
            &["stave-camera-button", "flat"],
        );
        let bridge = Bridge::<CameraComponent>::launch(CameraInit {
            fit: fit.clone(),
            front: front.clone(),
            turn: turn.clone(),
            section: tools.section.clone(),
            line: tools.line.clone(),
            popout: tools.popout.clone(),
        });
        CameraToolbar {
            root: bridge.widget().clone(),
            fit,
            front,
            turn,
            bridge,
        }
    }

    /// Lays the buttons out in a column (docked on the right edge) or a
    /// row (docked in a corner), X-ray and Pull apart with them. The
    /// column already holds `tools`.
    pub fn orientation_setter(&self, _tools: &ViewToolbar) -> impl Fn(gtk::Orientation) + 'static {
        let input = self.bridge.model().input.clone();
        move |orientation| input.emit(CameraInput::SetOrientation(orientation))
    }
}

/// The key shown while a problem is posed: a red square and "Shared wood
/// at first contact".
pub fn contact_key() -> gtk::Box {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Start)
        .margin_top(52)
        .css_classes(["stave-contact-key"])
        .visible(false)
        .build();
    root.append(&icons::swatch(
        gtk::gdk::RGBA::new(0.878, 0.106, 0.141, 1.0),
        false,
        10,
    ));
    root.append(&gtk::Label::new(Some("Shared wood at first contact")));
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
            set_halign: gtk::Align::End,
            set_valign: gtk::Align::Start,
            set_margin_top: 12,
            set_margin_end: 12,
            add_css_class: "stave-plan-switch",
            set_visible: false,
            update_property: &[gtk::accessible::Property::Label("Hinge view")],

            #[local_ref]
            from_above -> gtk::ToggleButton {
                #[watch]
                #[block_signal(from_above_toggled)]
                set_active: model.plan,
                connect_toggled[sender] => move |button| {
                    sender
                        .input_sender()
                        .emit(PlanSwitchInput::Toggled(button.is_active()));
                } @from_above_toggled,
            },
            #[local_ref]
            in_3d -> gtk::ToggleButton {
                // Turning it on turns "From above" off, which only says
                // what the model says already.
                #[watch]
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

    fn update(&mut self, message: PlanSwitchInput, sender: ComponentSender<Self>, _: &Self::Root) {
        match message {
            PlanSwitchInput::Show { plan } => self.plan = plan,
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
/// keeps the API the window still uses and owns the component. Setters
/// are queued.
pub struct PlanSwitch {
    /// The switch, to overlay on the view.
    pub root: gtk::Box,
    /// From above (plan).
    pub from_above: gtk::ToggleButton,
    /// In 3D.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "kept for the switch's API; only tests press it now"
        )
    )]
    pub in_3d: gtk::ToggleButton,
    bridge: Bridge<PlanSwitchComponent>,
}

impl PlanSwitch {
    /// The switch, in 3D.
    pub fn new() -> PlanSwitch {
        let toggle = |shape: icons::Shape, label: &str| {
            let button = gtk::ToggleButton::builder()
                .child(&PathIcon::new(shape, 17))
                .tooltip_text(label)
                .css_classes(["stave-plan-button"])
                .build();
            button.update_property(&[gtk::accessible::Property::Label(label)]);
            button
        };
        let from_above = toggle(icons::FROM_ABOVE, "From above");
        let in_3d = toggle(icons::PARTS, "In 3D");
        in_3d.set_group(Some(&from_above));
        let bridge = Bridge::<PlanSwitchComponent>::launch(PlanSwitchInit {
            from_above: from_above.clone(),
            in_3d: in_3d.clone(),
        });
        PlanSwitch {
            root: bridge.widget().clone(),
            from_above,
            in_3d,
            bridge,
        }
    }

    /// Shows which view is on.
    pub fn show(&self, plan: bool) {
        self.bridge.send(PlanSwitchInput::Show { plan });
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
            title: "Checking…".to_owned(),
            description: String::new(),
            offers_open: false,
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "kept for the page's API; only tests read it now")
    )]
    pub open: gtk::Button,
    bridge: Bridge<EmptyViewComponent>,
}

impl EmptyView {
    /// The page, saying "Checking…".
    pub fn new() -> EmptyView {
        let open = gtk::Button::builder()
            .label("Open a Project…")
            .action_name("win.open")
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let bridge = Bridge::<EmptyViewComponent>::launch(open.clone());
        EmptyView {
            root: bridge.widget().clone(),
            open,
            bridge,
        }
    }

    /// Says why there is nothing to draw.
    pub fn show(&self, title: &str, description: &str, offers_open: bool) {
        self.bridge.send(EmptyViewInput::Show {
            title: title.to_owned(),
            description: description.to_owned(),
            offers_open,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    #[test]
    fn a_dropped_view_toolbar_is_freed() {
        on_gtk_thread(view_toolbar_is_freed);
    }

    /// Dropping the toolbar shuts its component down: its widgets, its
    /// slider and the window's callback are freed.
    fn view_toolbar_is_freed() {
        let tools = ViewToolbar::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        tools.slider.connect_changed(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let popout = tools.popout.downgrade();
        let pull = tools.pull.downgrade();
        let slider = tools.bridge.model().slider.root.downgrade();
        let input = tools.bridge.sender().clone();
        settle();
        assert!(popout.upgrade().is_some(), "the tools live while held");
        drop(tools);
        settle();
        assert!(popout.upgrade().is_none(), "the pop-out is freed");
        assert!(pull.upgrade().is_none(), "Pull apart is freed");
        assert!(slider.upgrade().is_none(), "the slider is freed");
        assert!(input.send(ViewToolbarInput::Offer(true)).is_err());
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn showing_pull_apart_does_not_read_as_people_toggling_it() {
        on_gtk_thread(show_does_not_call_back);
    }

    /// The window used to raise `updating_controls` around `show` so the
    /// toggle's handler would ignore it; now the handler is blocked while
    /// the view sets it, so `show` and `offer` never send an output.
    fn show_does_not_call_back() {
        let tools = ViewToolbar::new();
        let heard: Rc<RefCell<Vec<ViewToolbarOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        tools
            .bridge
            .connect(move |output| log.borrow_mut().push(output));
        tools.show(true, 0.4, true);
        assert!(!tools.pull.is_active(), "show is queued");
        settle();
        assert!(tools.pull.is_active());
        assert!(tools.popout.is_visible(), "the slider pops out");
        assert_eq!(tools.bridge.model().slider.value(), f64::from(0.4_f32));
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
        assert!(heard.borrow().is_empty(), "setters never call back");
    }

    #[test]
    fn a_toggle_fired_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(toggles_are_queued);
    }

    /// People pressing Pull apart only queues a message; the output comes
    /// after the update. The window showing it off from inside that
    /// callback is queued too, and is not heard back as people's input.
    fn toggles_are_queued() {
        let tools = Rc::new(ViewToolbar::new());
        let heard: Rc<RefCell<Vec<ViewToolbarOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&tools);
        tools.bridge.connect(move |output| {
            let Some(tools) = weak.upgrade() else {
                return;
            };
            // Reading the model here would panic if the toolbar were
            // still in its update.
            let on = tools.bridge.model().on;
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
            *heard.borrow(),
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
        let tools = ViewToolbar::new();
        let camera = CameraToolbar::new(&tools);
        let orient = camera.orientation_setter(&tools);
        orient(gtk::Orientation::Horizontal);
        assert_eq!(
            camera.root.orientation(),
            gtk::Orientation::Vertical,
            "queued"
        );
        settle();
        assert_eq!(camera.root.orientation(), gtk::Orientation::Horizontal);
        assert_eq!(tools.section.orientation(), gtk::Orientation::Horizontal);
        assert_eq!(tools.line.orientation(), gtk::Orientation::Vertical);
        assert!(camera.root.has_css_class("flat-row"));
        tools.show(true, 0.0, false);
        settle();
        assert!(
            camera.root.has_css_class("joined"),
            "the pop-out is beside it"
        );
        tools.show(false, 0.0, false);
        settle();
        assert!(!camera.root.has_css_class("joined"));
    }

    #[test]
    fn camera_presses_are_heard_after_the_update_and_the_column_is_freed() {
        on_gtk_thread(camera_presses);
    }

    fn camera_presses() {
        let tools = ViewToolbar::new();
        let camera = CameraToolbar::new(&tools);
        let heard: Rc<RefCell<Vec<CameraOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        camera
            .bridge
            .connect(move |output| log.borrow_mut().push(output));
        camera.front.emit_clicked();
        camera.turn.emit_clicked();
        assert!(heard.borrow().is_empty(), "queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![CameraOutput::Front, CameraOutput::Turn]
        );
        let root = camera.root.downgrade();
        let input = camera.bridge.sender().clone();
        let orient = camera.orientation_setter(&tools);
        drop(camera);
        settle();
        assert!(root.upgrade().is_none(), "the column is freed");
        // The pop-out outlives the column, no longer telling it.
        tools.show(true, 0.0, false);
        settle();
        assert!(tools.popout.is_visible());
        drop(tools);
        assert!(input.send(CameraInput::Joined(false)).is_err());
        // A setter outliving the column does nothing.
        orient(gtk::Orientation::Vertical);
    }

    #[test]
    fn the_plan_switch_hears_people_not_itself() {
        on_gtk_thread(plan_switch);
    }

    /// Showing the view from above sets the toggles with the handler
    /// blocked (or says what the model already says), so only people's
    /// presses are heard, each after its update.
    fn plan_switch() {
        let switch = Rc::new(PlanSwitch::new());
        settle();
        assert!(switch.in_3d.is_active(), "it opens in 3D");
        let heard: Rc<RefCell<Vec<PlanSwitchOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&switch);
        switch.bridge.connect(move |output| {
            log.borrow_mut().push(output);
            if let Some(switch) = weak.upgrade() {
                // The window shows it back from inside its callback.
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
    fn the_empty_view_says_why_and_is_freed() {
        on_gtk_thread(empty_view);
    }

    fn empty_view() {
        let empty = EmptyView::new();
        settle();
        assert_eq!(empty.bridge.model().title, "Checking…");
        assert!(!empty.open.is_visible());
        empty.show("A library folder", "Open a project to see it.", true);
        settle();
        assert_eq!(empty.bridge.model().title, "A library folder");
        assert!(empty.open.is_visible());
        let root = empty.root.downgrade();
        let input = empty.bridge.sender().clone();
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
