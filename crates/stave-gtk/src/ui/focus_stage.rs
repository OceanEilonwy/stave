//! What focus mode and a selected problem put in the 3D view's area
//! (JointFocus.dc.html, Hinge.dc.html, Problems.dc.html):
//!
//! - the joint's step player: play, "Step 1 of 1 · push(rung, along:
//!   length)", how far the part is from home, and the apart … together
//!   scrub;
//! - for hardware, the line saying what the view looks at, three small
//!   views at fixed angles, and the swing card (play, the door's name, the
//!   slider and its angle);
//! - a problem's motion card: "Swing door to see where it hits", the
//!   slider with the blocked range hatched and the handle red past the
//!   contact.
//!
//! Each is a relm4 component (docs/relm4-migration.md): its model is the
//! only state, only its `update` changes it, and its widgets' handlers
//! only send it messages. The structs named after them ([`StepCard`],
//! [`SwingCard`], [`Caption`], [`Pictures`], [`ProblemCard`]) are the
//! window's handles: each is the one owner of its component's controller,
//! its outputs forwarded at launch to where the window said, its setters
//! sent as input messages. The window sets their values and hears back
//! only from what people do. A card's slider forwards its moves to the
//! card's own input.

use adw::prelude::*;
use gtk::gdk;
use relm4::{
    Component, ComponentController, ComponentParts, ComponentSender, Controller, RelmWidgetExt,
};
use stave_app::i18n::{fill_in, tr};
use stave_app::problems::MotionCard;

use super::icons::{self, PathIcon};
use super::slider::Slider;
use super::viewport::{Sizing, Viewport};

/// A round blue play button, 40 px, its icon and its words drawn from
/// whether it plays: see [`play_shape`] and [`play_text`].
fn play_button() -> (gtk::Button, PathIcon) {
    let icon = PathIcon::new(icons::PLAY, 14);
    let button = gtk::Button::builder()
        .child(&icon)
        .css_classes(["stave-play", "circular"])
        .valign(gtk::Align::Center)
        .build();
    (button, icon)
}

/// A play button's icon: pause while it plays.
fn play_shape(playing: bool) -> icons::Shape {
    if playing { icons::PAUSE } else { icons::PLAY }
}

/// A play button's tooltip and what screen readers call it: "Pause" while
/// it plays, else what it plays (`label`).
fn play_text(playing: bool, label: &str) -> String {
    if playing {
        tr("Pause")
    } else {
        label.to_owned()
    }
}

/// The step line's markup: the lead in bold, the call in monospace.
fn step_markup(step: Option<&str>) -> String {
    let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
    match step.map(|step| step.split_once(" · ").unwrap_or((step, ""))) {
        Some((lead, "")) => format!("<b>{}</b>", escape(lead)),
        Some((lead, text)) => format!(
            "<b>{}</b> · <span font_family=\"monospace\" size=\"92%\">{}</span>",
            escape(lead),
            escape(text)
        ),
        None => String::new(),
    }
}

/// The joint's step player: what it shows, and its slider.
pub struct StepCardComponent {
    /// The step line, as markup.
    markup: String,
    /// The step line in full, for its tooltip.
    tooltip: Option<String>,
    /// How far the part is from home.
    status: String,
    /// It plays: its button shows pause.
    playing: bool,
    /// Apart (0) to together (1).
    scrub: Slider,
}

/// What a step player is told.
#[derive(Debug)]
pub enum StepCardInput {
    /// Show the step ("Step 1 of 1 · push(rung, along: length)") and the
    /// distance from home.
    ShowFrame {
        /// The step, if there is one.
        step: Option<String>,
        /// The distance from home.
        status: String,
    },
    /// Move the slider, without telling anyone.
    SetProgress(f64),
    /// Show play, or pause while it plays.
    ShowPlaying(bool),
    /// People moved the slider.
    Scrubbed(f64),
    /// People pressed play.
    PlayPressed,
}

/// What a step player tells its owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StepCardOutput {
    /// People pressed play (or pause).
    Play,
    /// People moved the slider to this progress.
    Scrubbed(f64),
}

#[relm4::component(pub)]
impl Component for StepCardComponent {
    type Init = ();
    type Input = StepCardInput;
    type Output = StepCardOutput;
    type CommandOutput = ();

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 14,
            // It floats on the 3D view, placed exactly by the floats.
            add_css_class: "stave-focus-card",
            set_visible: false,
            update_property: &[gtk::accessible::Property::Label(&tr("Assembly steps"))],

            #[local_ref]
            play -> gtk::Button {
                #[watch]
                set_tooltip_text: Some(&play_text(model.playing, &tr("Play the assembly"))),
                #[watch]
                update_property: &[gtk::accessible::Property::Label(
                    &play_text(model.playing, &tr("Play the assembly")),
                )],
                connect_clicked[sender] => move |_| {
                    sender.input_sender().emit(StepCardInput::PlayPressed);
                },
            },
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 6,
                set_hexpand: true,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Label {
                        set_use_markup: true,
                        set_xalign: 0.0,
                        set_hexpand: true,
                        set_ellipsize: gtk::pango::EllipsizeMode::End,
                        #[watch]
                        set_markup: &model.markup,
                        #[watch]
                        set_tooltip_text: model.tooltip.as_deref(),
                    },
                    gtk::Label {
                        set_xalign: 1.0,
                        add_css_class: "stave-card-dim",
                        #[watch]
                        set_label: &model.status,
                    },
                },
                #[local_ref]
                scrub_area -> gtk::DrawingArea {},
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,

                    gtk::Label {
                        set_label: "apart",
                        set_hexpand: true,
                        set_xalign: 0.0,
                        add_css_class: "stave-card-dim",
                    },
                    gtk::Label {
                        set_label: "together",
                        add_css_class: "stave-card-dim",
                    },
                },
            },
        },

        #[local_ref]
        play_icon -> PathIcon {
            #[watch]
            set_shape: play_shape(model.playing),
        },
    }

    fn init(
        (): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (play, play_icon) = play_button();
        let (play, play_icon) = (&play, &play_icon);
        let scrub = Slider::launch(
            &tr("Apart to together"),
            22,
            22.0,
            2.0,
            sender.input_sender(),
            StepCardInput::Scrubbed,
        );
        let scrub_area = &scrub.root.clone();
        let model = StepCardComponent {
            markup: String::new(),
            tooltip: None,
            status: String::new(),
            playing: false,
            scrub,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: StepCardInput, sender: ComponentSender<Self>, _: &Self::Root) {
        match message {
            StepCardInput::ShowFrame { step, status } => {
                self.markup = step_markup(step.as_deref());
                self.tooltip = step;
                self.status = status;
            }
            StepCardInput::SetProgress(value) => self.scrub.set_value(value),
            StepCardInput::ShowPlaying(playing) => self.playing = playing,
            StepCardInput::Scrubbed(value) => {
                // Nobody listening is not a mistake.
                let _ = sender.output(StepCardOutput::Scrubbed(value));
            }
            StepCardInput::PlayPressed => {
                let _ = sender.output(StepCardOutput::Play);
            }
        }
    }
}

/// The joint's step player, floating at the bottom of the view. It owns
/// the component: dropping it shuts the component down. Setters are
/// queued; people's presses and moves reach the window as
/// [`StepCardOutput`]s.
pub struct StepCard {
    /// The card.
    pub root: gtk::Box,
    controller: Controller<StepCardComponent>,
}

impl StepCard {
    /// A hidden card, people's presses and moves made into the owner's
    /// messages by `map` and sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(StepCardOutput) -> X + 'static,
    ) -> StepCard {
        let controller = StepCardComponent::builder().launch(()).forward(to, map);
        StepCard {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Queues `input`; once the card has gone there is nothing to show.
    fn send(&self, input: StepCardInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Moves the slider (apart 0 to together 1), without calling back.
    pub fn set_progress(&self, progress: f64) {
        self.send(StepCardInput::SetProgress(progress));
    }

    /// Shows play, or pause while the joint plays.
    pub fn show_playing(&self, playing: bool) {
        self.send(StepCardInput::ShowPlaying(playing));
    }

    /// Shows the step ("Step 1 of 1 · push(rung, along: length)") and the
    /// distance from home.
    pub fn show_frame(&self, step: Option<&str>, status: &str) {
        self.send(StepCardInput::ShowFrame {
            step: step.map(str::to_owned),
            status: status.to_owned(),
        });
    }
}

/// Hardware focus's swing (or slide) card: what it shows, and its slider.
pub struct SwingCardComponent {
    /// What moves.
    name: String,
    /// The reading: "45°", "250 mm".
    value: String,
    /// It plays: its button shows pause.
    playing: bool,
    /// Closed (0) to fully open (1).
    scrub: Slider,
}

/// What a swing card is told.
#[derive(Debug)]
pub enum SwingCardInput {
    /// Name what moves, and whether it turns (a door) or slides (a drawer).
    ShowName {
        /// What moves.
        name: String,
        /// It turns.
        turns: bool,
    },
    /// Show the reading: "45°", "250 mm".
    ShowValue(String),
    /// Move the slider, without telling anyone.
    SetProgress(f64),
    /// Show play, or pause while it plays.
    ShowPlaying(bool),
    /// People moved the slider.
    Scrubbed(f64),
    /// People pressed play.
    PlayPressed,
}

/// What a swing card tells its owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwingCardOutput {
    /// People pressed play (or pause).
    Play,
    /// People moved the slider to this progress.
    Scrubbed(f64),
}

#[relm4::component(pub)]
impl Component for SwingCardComponent {
    type Init = ();
    type Input = SwingCardInput;
    type Output = SwingCardOutput;
    type CommandOutput = ();

    view! {
        #[root]
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 14,
            add_css_class: "stave-focus-card",
            set_visible: false,
            update_property: &[gtk::accessible::Property::Label(&tr("Swing the door"))],

            #[local_ref]
            play -> gtk::Button {
                #[watch]
                set_tooltip_text: Some(&play_text(model.playing, &tr("Play the swing"))),
                #[watch]
                update_property: &[gtk::accessible::Property::Label(
                    &play_text(model.playing, &tr("Play the swing")),
                )],
                connect_clicked[sender] => move |_| {
                    sender.input_sender().emit(SwingCardInput::PlayPressed);
                },
            },
            gtk::Label {
                add_css_class: "heading",
                add_css_class: "stave-card-name",
                #[watch]
                set_label: &model.name,
            },
            #[local_ref]
            scrub_area -> gtk::DrawingArea {},
            gtk::Label {
                set_xalign: 1.0,
                set_width_chars: 6,
                add_css_class: "stave-card-value",
                #[watch]
                set_label: &model.value,
            },
        },

        #[local_ref]
        play_icon -> PathIcon {
            #[watch]
            set_shape: play_shape(model.playing),
        },
    }

    fn init(
        (): Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (play, play_icon) = play_button();
        let (play, play_icon) = (&play, &play_icon);
        let scrub = Slider::launch(
            &tr("Door angle"),
            22,
            22.0,
            2.0,
            sender.input_sender(),
            SwingCardInput::Scrubbed,
        );
        let scrub_area = &scrub.root.clone();
        let model = SwingCardComponent {
            name: String::new(),
            value: String::new(),
            playing: false,
            scrub,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: SwingCardInput, sender: ComponentSender<Self>, _: &Self::Root) {
        match message {
            SwingCardInput::ShowName { name, turns } => {
                self.name = name;
                let label = if turns {
                    tr("Door angle")
                } else {
                    tr("How far it slides out")
                };
                self.scrub.set_label(&label);
            }
            SwingCardInput::ShowValue(text) => {
                self.scrub.set_value_text(&text);
                self.value = text;
            }
            SwingCardInput::SetProgress(value) => self.scrub.set_value(value),
            SwingCardInput::ShowPlaying(playing) => self.playing = playing,
            SwingCardInput::Scrubbed(value) => {
                let _ = sender.output(SwingCardOutput::Scrubbed(value));
            }
            SwingCardInput::PlayPressed => {
                let _ = sender.output(SwingCardOutput::Play);
            }
        }
    }
}

/// Hardware focus: the swing (or slide) card under the small views. It
/// owns the component: dropping it shuts the component down. Setters are
/// queued; people's presses and moves reach the window as
/// [`SwingCardOutput`]s.
pub struct SwingCard {
    /// The card.
    pub root: gtk::Box,
    controller: Controller<SwingCardComponent>,
}

impl SwingCard {
    /// A hidden card, people's presses and moves made into the owner's
    /// messages by `map` and sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(SwingCardOutput) -> X + 'static,
    ) -> SwingCard {
        let controller = SwingCardComponent::builder().launch(()).forward(to, map);
        SwingCard {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Queues `input`; once the card has gone there is nothing to show.
    fn send(&self, input: SwingCardInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Moves the slider (closed 0 to fully open 1), without calling back.
    pub fn set_progress(&self, progress: f64) {
        self.send(SwingCardInput::SetProgress(progress));
    }

    /// Shows play, or pause while the door plays.
    pub fn show_playing(&self, playing: bool) {
        self.send(SwingCardInput::ShowPlaying(playing));
    }

    /// Names what moves.
    pub fn show_name(&self, name: &str, turns: bool) {
        self.send(SwingCardInput::ShowName {
            name: name.to_owned(),
            turns,
        });
    }

    /// Shows the reading: "45°", "250 mm".
    pub fn show_value(&self, text: &str) {
        self.send(SwingCardInput::ShowValue(text.to_owned()));
    }
}

/// The caption's markup: the first two words in bold, as the design has
/// it.
fn caption_markup(text: &str) -> String {
    let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
    let mut words = text.splitn(3, ' ');
    match (words.next(), words.next(), words.next()) {
        (Some(a), Some(b), Some(rest)) => {
            format!("<b>{} {}</b> {}", escape(a), escape(b), escape(rest))
        }
        _ => format!("<b>{}</b>", escape(text)),
    }
}

/// The line over hardware focus's view: what it shows.
#[derive(Debug, Default)]
pub struct CaptionComponent {
    /// What the view looks at, as markup.
    markup: String,
    /// Which way is which, at the end.
    hint: String,
}

/// What the caption is told.
#[derive(Debug)]
pub enum CaptionInput {
    /// "**Looking down** on the hinge, through the cabinet top" and a hint
    /// at the end.
    Show {
        /// What the view looks at.
        text: String,
        /// Which way is which ("back ↑ · front ↓"); empty for none.
        hint: String,
    },
}

#[relm4::component(pub)]
impl Component for CaptionComponent {
    type Init = ();
    type Input = CaptionInput;
    type Output = ();
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Horizontal,
            set_spacing: 12,
            add_css_class: "stave-focus-caption",
            set_visible: false,

            gtk::Label {
                set_use_markup: true,
                set_xalign: 0.0,
                set_hexpand: true,
                set_ellipsize: gtk::pango::EllipsizeMode::End,
                #[watch]
                set_markup: &model.markup,
            },
            gtk::Label {
                add_css_class: "stave-card-dim",
                set_ellipsize: gtk::pango::EllipsizeMode::Start,
                #[watch]
                set_label: &model.hint,
                #[watch]
                set_visible: !model.hint.is_empty(),
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = CaptionComponent::default();
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: CaptionInput, _: ComponentSender<Self>, _: &Self::Root) {
        match message {
            CaptionInput::Show { text, hint } => {
                self.markup = caption_markup(&text);
                self.hint = hint;
            }
        }
    }
}

/// The line over hardware focus's view: what it looks at and which way is
/// which. It owns the component's controller; setters are queued.
pub struct Caption {
    /// The line.
    pub root: gtk::Box,
    controller: Controller<CaptionComponent>,
}

impl Caption {
    /// A hidden line.
    pub fn new() -> Caption {
        // It tells nobody anything.
        let controller = CaptionComponent::builder().launch(()).detach();
        Caption {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// "**Looking down** on the hinge, through the cabinet top" and a hint
    /// at the end.
    pub fn show(&self, text: &str, hint: &str) {
        let _ = self.controller.sender().send(CaptionInput::Show {
            text: text.to_owned(),
            hint: hint.to_owned(),
        });
    }
}

/// Hardware focus's three small views: their slots and their size.
pub struct PicturesComponent {
    /// The slots, one a picture, in order, with their captions.
    slots: Vec<(Viewport, String)>,
    /// The size in device pixels to render the pictures at, as last
    /// told to the owner; `None` before the slots have room.
    size: Option<(u32, u32)>,
}

/// What the small views are told.
#[derive(Debug)]
pub enum PicturesInput {
    /// Lay out a picture for each caption, empty until
    /// [`PicturesInput::ShowTextures`]. The same captions as last time
    /// keep the pictures as they are.
    Show(Vec<String>),
    /// Show the rendered pictures, in the order of the captions.
    ShowTextures(Vec<gdk::Texture>),
    /// Ring the picture at this index.
    ShowCurrent(Option<usize>),
    /// A picture's size in device pixels changed.
    SlotResized {
        /// Its width in device pixels.
        width: u32,
        /// Its height in device pixels.
        height: u32,
    },
    /// People pressed the picture at this index.
    Pressed(usize),
}

/// What the small views tell their owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PicturesOutput {
    /// The size in device pixels to render them at changed (`Some`: they
    /// need rendering again), or they were laid out afresh and have no
    /// room yet (`None`: nothing to render until they do).
    Resized(Option<(u32, u32)>),
    /// People chose the picture at this index: go to its pose.
    Chosen(usize),
}

impl PicturesComponent {
    /// Builds a picture for each caption in `root`, in place of the old.
    fn lay_out(&mut self, root: &gtk::Box, captions: Vec<String>, sender: &ComponentSender<Self>) {
        while let Some(child) = root.first_child() {
            root.remove(&child);
        }
        if self.size.take().is_some() {
            let _ = sender.output(PicturesOutput::Resized(None));
        }
        let count = captions.len();
        let mut slots = Vec::with_capacity(count);
        for (index, caption) in captions.into_iter().enumerate() {
            let picture = Viewport::picture(Sizing::Picture {
                natural_width: 320,
                minimum_height: 110,
            });
            let input = sender.input_sender().clone();
            picture.connect_resized(move |width, height| {
                input.emit(PicturesInput::SlotResized { width, height });
            });
            let template = if index == 0 {
                tr("{caption} closed")
            } else if index + 1 == count {
                tr("{caption} fully open")
            } else {
                tr("{caption} part way")
            };
            let caption_markup = format!("<b>{}</b>", gtk::glib::markup_escape_text(&caption));
            let label = gtk::Label::builder()
                .use_markup(true)
                .label(fill_in(&template, &[("caption", &caption_markup)]))
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["stave-picture-caption"])
                .build();
            let cell = gtk::Box::new(gtk::Orientation::Vertical, 4);
            cell.append(&picture);
            cell.append(&label);
            let button = gtk::Button::builder()
                .child(&cell)
                .css_classes(["stave-picture"])
                .tooltip_text(fill_in(
                    &tr("Show it at {caption}"),
                    &[("caption", &caption)],
                ))
                .build();
            let input = sender.input_sender().clone();
            button.connect_clicked(move |_| input.emit(PicturesInput::Pressed(index)));
            root.append(&button);
            slots.push((picture, caption));
        }
        self.slots = slots;
    }
}

impl Component for PicturesComponent {
    type Init = ();
    type Input = PicturesInput;
    type Output = PicturesOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> gtk::Box {
        gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .homogeneous(true)
            .css_classes(["stave-pictures"])
            .visible(false)
            .build()
    }

    fn init(_: (), _: gtk::Box, _: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = PicturesComponent {
            slots: Vec::new(),
            size: None,
        };
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: PicturesInput, sender: ComponentSender<Self>, root: &gtk::Box) {
        match message {
            PicturesInput::Show(captions) => {
                let unchanged = self
                    .slots
                    .iter()
                    .map(|(_, caption)| caption)
                    .eq(captions.iter());
                if !unchanged {
                    self.lay_out(root, captions, &sender);
                }
            }
            PicturesInput::ShowTextures(textures) => {
                for ((slot, _), texture) in self.slots.iter().zip(textures) {
                    slot.show_frame(texture);
                }
            }
            PicturesInput::ShowCurrent(current) => {
                let mut child = root.first_child();
                let mut index = 0;
                while let Some(widget) = child {
                    widget.set_class_active("current", current == Some(index));
                    index += 1;
                    child = widget.next_sibling();
                }
            }
            PicturesInput::SlotResized { width, height } => {
                // Every slot is the same size: the first to report says
                // it, once the first slot has been given room.
                let roomy = self
                    .slots
                    .first()
                    .is_some_and(|(slot, _)| slot.width() > 1 && slot.height() > 1);
                let size = roomy.then_some((width, height));
                if self.size != size {
                    self.size = size;
                    let _ = sender.output(PicturesOutput::Resized(size));
                }
            }
            PicturesInput::Pressed(index) => {
                let _ = sender.output(PicturesOutput::Chosen(index));
            }
        }
    }
}

/// Hardware focus's three small views at fixed angles. Each is rendered at
/// its slot's size in device pixels and drawn one texel to a pixel, so it
/// is sharp at any size and scale. Setters are queued; the window hears
/// [`PicturesOutput`]s.
pub struct Pictures {
    /// The row.
    pub root: gtk::Box,
    /// The owner's copy of the size to render at, as the row last
    /// published it ([`PicturesOutput::Resized`], handed back with
    /// [`Pictures::keep_size`]); never read from the row itself.
    size: Option<(u32, u32)>,
    controller: Controller<PicturesComponent>,
}

impl Pictures {
    /// A hidden row. When the pictures' size in device pixels changes (the
    /// window resized, or moved to a screen of another scale: they need
    /// rendering again at the size it carries) and when people choose
    /// one, `map` makes it the owner's message, sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(PicturesOutput) -> X + 'static,
    ) -> Pictures {
        let controller = PicturesComponent::builder().launch(()).forward(to, map);
        Pictures {
            root: controller.widget().clone(),
            size: None,
            controller,
        }
    }

    /// Queues `input`; once the row has gone there is nothing to show.
    fn send(&self, input: PicturesInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Keeps the size the row published ([`PicturesOutput::Resized`]),
    /// the owner's copy to render at.
    pub fn keep_size(&mut self, size: Option<(u32, u32)>) {
        self.size = size;
    }

    /// The size, in device pixels, to render the pictures at, as the row
    /// last published it; `None` before they have been given room.
    pub fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    /// Lays out a picture for each caption, each a button that is heard
    /// as [`PicturesOutput::Chosen`] with its index, empty until
    /// [`Pictures::show_textures`]. The same captions as last time keep
    /// the pictures as they are.
    pub fn show(&self, captions: &[String]) {
        self.send(PicturesInput::Show(captions.to_vec()));
    }

    /// Shows the rendered pictures, in the order of the captions.
    pub fn show_textures(&self, textures: Vec<gdk::Texture>) {
        self.send(PicturesInput::ShowTextures(textures));
    }

    /// Rings the picture at `current`.
    pub fn show_current(&self, current: Option<usize>) {
        self.send(PicturesInput::ShowCurrent(current));
    }
}

/// A selected problem's motion card: what it shows, and its slider.
pub struct ProblemCardComponent {
    /// A motion is posed: the card shows.
    shown: bool,
    /// "**Swing door** to see where it hits", as markup.
    lead: String,
    /// The reading at the slider.
    value: String,
    /// The reading is past the contact: it shows red.
    past: bool,
    /// The words under the slider's start, blocked range and end.
    start: String,
    blocked: String,
    end: String,
    /// Closed to fully open, in degrees or millimetres.
    scrub: Slider,
}

/// What a problem's motion card is told.
#[derive(Debug)]
pub enum ProblemCardInput {
    /// Show the card for a motion, the slider at `amount` (radians or
    /// millimetres); no card hides it.
    Show {
        /// The motion, if one is posed.
        card: Option<MotionCard>,
        /// Where the slider is.
        amount: f32,
    },
    /// Show the reading at `amount`: red once it is past the contact.
    ShowValue {
        /// The motion.
        card: MotionCard,
        /// Where the slider is.
        amount: f32,
    },
    /// People moved the slider.
    Moved(f64),
}

/// What a problem's motion card tells its owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProblemCardOutput {
    /// People moved the slider to this value (degrees or millimetres).
    Moved(f64),
}

impl ProblemCardComponent {
    /// Takes the reading at `amount`.
    fn show_value(&mut self, card: &MotionCard, amount: f32) {
        let text = card.amount_text(amount);
        self.scrub.set_value_text(&text);
        self.value = text;
        self.past = amount >= card.contact();
    }
}

#[relm4::component(pub)]
impl Component for ProblemCardComponent {
    type Init = ();
    type Input = ProblemCardInput;
    type Output = ProblemCardOutput;
    type CommandOutput = ();

    view! {
        adw::Clamp {
            set_maximum_size: 620,
            set_tightening_threshold: 620,
            // It floats on the 3D view, placed exactly by the floats.
            #[watch]
            set_visible: model.shown,

            #[wrap(Some)]
            set_child = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
                add_css_class: "stave-focus-card",
                add_css_class: "stave-problem-card",

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,

                    gtk::Label {
                        set_use_markup: true,
                        set_xalign: 0.0,
                        set_hexpand: true,
                        #[watch]
                        set_markup: &model.lead,
                    },
                    gtk::Label {
                        add_css_class: "stave-card-value",
                        #[watch]
                        set_label: &model.value,
                        #[watch]
                        set_class_active: ("blocked", model.past),
                    },
                },
                #[local_ref]
                scrub_area -> gtk::DrawingArea {},
                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_homogeneous: true,

                    gtk::Label {
                        add_css_class: "stave-card-dim",
                        set_xalign: 0.0,
                        set_hexpand: true,
                        #[watch]
                        set_label: &model.start,
                    },
                    gtk::Label {
                        add_css_class: "stave-card-blocked",
                        set_hexpand: true,
                        #[watch]
                        set_label: &model.blocked,
                    },
                    gtk::Label {
                        add_css_class: "stave-card-dim",
                        set_xalign: 1.0,
                        set_hexpand: true,
                        #[watch]
                        set_label: &model.end,
                    },
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let scrub = Slider::launch(
            &tr("Swing it"),
            26,
            22.0,
            2.0,
            sender.input_sender(),
            ProblemCardInput::Moved,
        );
        let scrub_area = &scrub.root.clone();
        let model = ProblemCardComponent {
            shown: false,
            lead: String::new(),
            value: String::new(),
            past: false,
            start: String::new(),
            blocked: String::new(),
            end: String::new(),
            scrub,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: ProblemCardInput, sender: ComponentSender<Self>, _: &Self::Root) {
        match message {
            ProblemCardInput::Show { card: None, .. } => self.shown = false,
            ProblemCardInput::Show {
                card: Some(card),
                amount,
            } => {
                let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
                let lead = card.lead();
                self.lead = fill_in(
                    &tr("{name} <span alpha=\"70%\">to see where it hits</span>"),
                    &[("name", &format!("<b>{}</b>", escape(&lead)))],
                );
                let (full, contact, value) = if card.turns() {
                    (
                        f64::from(card.full()).to_degrees(),
                        f64::from(card.contact()).to_degrees(),
                        f64::from(amount).to_degrees(),
                    )
                } else {
                    (
                        f64::from(card.full()),
                        f64::from(card.contact()),
                        f64::from(amount),
                    )
                };
                self.scrub.set_range(0.0, full.max(0.1));
                self.scrub.set_blocked(Some((contact, &card.blocked)));
                self.scrub.set_value(value);
                self.scrub.set_label(&lead);
                self.start.clone_from(&card.start);
                self.blocked.clone_from(&card.blocked);
                self.end.clone_from(&card.end);
                self.show_value(&card, amount);
                self.shown = true;
            }
            ProblemCardInput::ShowValue { card, amount } => self.show_value(&card, amount),
            ProblemCardInput::Moved(value) => {
                let _ = sender.output(ProblemCardOutput::Moved(value));
            }
        }
    }
}

/// A selected problem's motion card: swing the door to see where it hits.
/// It owns the component. Setters are queued; people's swings of its
/// slider (closed to fully open, in degrees or millimetres) reach the
/// window as [`ProblemCardOutput::Moved`].
pub struct ProblemCard {
    /// The card, to overlay on the view.
    pub root: adw::Clamp,
    controller: Controller<ProblemCardComponent>,
}

impl ProblemCard {
    /// A hidden card, people's swings made into the owner's messages by
    /// `map` and sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(ProblemCardOutput) -> X + 'static,
    ) -> ProblemCard {
        let controller = ProblemCardComponent::builder().launch(()).forward(to, map);
        ProblemCard {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Queues `input`; once the card has gone there is nothing to show.
    fn send(&self, input: ProblemCardInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Shows the card for a motion, the slider at `amount` (radians or
    /// millimetres); `None` hides it.
    pub fn show(&self, card: Option<&MotionCard>, amount: f32) {
        self.send(ProblemCardInput::Show {
            card: card.cloned(),
            amount,
        });
    }

    /// Shows the reading at `amount`: red once it is past the contact.
    pub fn show_value(&self, card: &MotionCard, amount: f32) {
        self.send(ProblemCardInput::ShowValue {
            card: card.clone(),
            amount,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use stave_app::problems::MotionTravel;
    use stave_lang::units::Angle;
    use stave_model::ids::AssemblyId;

    use super::*;
    use crate::ui::gtk_test::{listener, on_gtk_thread, recorder, settle};

    /// Fires a press on a slider `x` pixels across, as GTK would.
    fn press(slider: &gtk::DrawingArea, x: f64) {
        let controllers = slider.observe_controllers();
        let drag = (0..controllers.n_items())
            .filter_map(|index| controllers.item(index))
            .find_map(|item| item.downcast::<gtk::GestureDrag>().ok());
        let Some(drag) = drag else {
            panic!("the slider has a drag controller");
        };
        drag.emit_by_name::<()>("drag-begin", &[&x, &0.0_f64]);
    }

    /// A callback's captures, to see them freed: a strong token for the
    /// callback and a weak one to watch.
    fn token() -> (Rc<()>, std::rc::Weak<()>) {
        let token = Rc::new(());
        let weak = Rc::downgrade(&token);
        (token, weak)
    }

    #[test]
    fn a_dropped_step_card_is_freed() {
        on_gtk_thread(step_card_is_freed);
    }

    /// The old card's slider kept its own state in shared cells; now
    /// dropping the card shuts its component down when the main loop next
    /// runs, and that frees the card, its slider and the window's
    /// callback.
    fn step_card_is_freed() {
        let (held, callback) = token();
        let to = listener(move |_: StepCardOutput| {
            let _ = &held;
        });
        let card = StepCard::new(&to, |output| output);
        drop(to);
        let root = card.root.downgrade();
        let slider = card.controller.model().scrub.root.downgrade();
        let input = card.controller.sender().clone();
        settle();
        assert!(root.upgrade().is_some(), "the card lives while held");
        drop(card);
        settle();
        assert!(root.upgrade().is_none(), "the card is freed");
        assert!(slider.upgrade().is_none(), "its slider is freed");
        assert!(
            input.send(StepCardInput::PlayPressed).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn a_step_cards_setters_do_not_call_back() {
        on_gtk_thread(step_card_setters);
    }

    fn step_card_setters() {
        let (to, heard) = recorder();
        let card = StepCard::new(&to, |output| output);
        card.show_frame(Some("Step 1 of 1 · push(rung, along: length)"), "6 mm");
        card.set_progress(0.5);
        assert_eq!(card.controller.model().status, "", "setters are queued");
        settle();
        let model = card.controller.model();
        assert_eq!(
            model.markup,
            "<b>Step 1 of 1</b> · <span font_family=\"monospace\" size=\"92%\">\
             push(rung, along: length)</span>"
        );
        assert_eq!(model.status, "6 mm");
        assert_eq!(model.scrub.value(), 0.5, "the slider moved");
        assert!(heard.borrow().is_empty(), "setters never call back");
    }

    #[test]
    fn a_press_on_the_step_cards_slider_is_queued_not_re_entered() {
        on_gtk_thread(step_card_presses_are_queued);
    }

    /// People's press on the slider reaches the window only after both
    /// the slider and the card have handled it, never from inside either;
    /// a press fired from inside the window's callback is queued too.
    fn step_card_presses_are_queued() {
        // The card, for the window's handler to read.
        let shared: Rc<RefCell<Option<std::rc::Weak<StepCard>>>> = Rc::default();
        let slider_shared: Rc<RefCell<Option<gtk::DrawingArea>>> = Rc::default();
        let heard: Rc<RefCell<Vec<f64>>> = Rc::default();
        let (log, weak, inner) = (
            Rc::clone(&heard),
            Rc::clone(&shared),
            Rc::clone(&slider_shared),
        );
        let to = listener(move |output| {
            let StepCardOutput::Scrubbed(value) = output else {
                return;
            };
            let Some(card) = weak.borrow().as_ref().and_then(std::rc::Weak::upgrade) else {
                return;
            };
            // Reading the model here would panic if the card were still
            // in its update.
            let _ = card.controller.model().status.len();
            if log.borrow().is_empty()
                && let Some(slider) = inner.borrow().as_ref()
            {
                press(slider, 0.0);
            }
            log.borrow_mut().push(value);
        });
        let card = Rc::new(StepCard::new(&to, |output| output));
        let slider = card.controller.model().scrub.root.clone();
        shared.replace(Some(Rc::downgrade(&card)));
        slider_shared.replace(Some(slider.clone()));
        // An unrealised slider is 0 px wide: a press past the handle is
        // the upper end.
        press(&slider, 1000.0);
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![1.0, 0.0],
            "the inner press came after"
        );
    }

    #[test]
    fn a_dropped_swing_card_is_freed_and_its_setters_do_not_call_back() {
        on_gtk_thread(swing_card);
    }

    fn swing_card() {
        let (held, callback) = token();
        let calls = Rc::new(Cell::new(0));
        let counted = Rc::clone(&calls);
        let to = listener(move |_: SwingCardOutput| {
            let _ = &held;
            counted.set(counted.get() + 1);
        });
        let card = SwingCard::new(&to, |output| output);
        drop(to);
        card.show_name("door", true);
        card.show_value("45°");
        card.set_progress(0.25);
        settle();
        {
            let model = card.controller.model();
            assert_eq!(model.name, "door");
            assert_eq!(model.value, "45°");
            assert_eq!(model.scrub.value(), 0.25);
        }
        assert_eq!(calls.get(), 0, "setters never call back");
        let root = card.root.downgrade();
        let input = card.controller.sender().clone();
        drop(card);
        settle();
        assert!(root.upgrade().is_none(), "the card is freed");
        assert!(input.send(SwingCardInput::PlayPressed).is_err());
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn the_swing_cards_play_button_is_heard_after_its_update() {
        on_gtk_thread(swing_play_is_queued);
    }

    fn swing_play_is_queued() {
        let (to, heard) = recorder();
        let card = SwingCard::new(&to, |output| output);
        play_of(&card.root).emit_clicked();
        assert!(heard.borrow().is_empty(), "the press is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![SwingCardOutput::Play]);
    }

    /// A card's play button: its first child.
    fn play_of(card: &gtk::Box) -> gtk::Button {
        let Some(play) = card
            .first_child()
            .and_then(|child| child.downcast::<gtk::Button>().ok())
        else {
            panic!("a card starts with its play button");
        };
        play
    }

    /// The icon a play button shows.
    fn shape_of(play: &gtk::Button) -> icons::Shape {
        let Some(icon) = play
            .child()
            .and_then(|child| child.downcast::<PathIcon>().ok())
        else {
            panic!("a play button shows an icon");
        };
        icon.shape()
    }

    #[test]
    fn the_cards_draw_play_or_pause_themselves() {
        on_gtk_thread(cards_show_playing);
    }

    /// The window used to set the play buttons' icon and words itself;
    /// now it tells each card whether it plays and the card draws it.
    fn cards_show_playing() {
        let (to, heard) = recorder::<StepCardOutput>();
        let step = StepCard::new(&to, |output| output);
        let (swing_to, swing_heard) = recorder::<SwingCardOutput>();
        let swing = SwingCard::new(&swing_to, |output| output);
        let (step_play, swing_play) = (play_of(&step.root), play_of(&swing.root));
        assert_eq!(shape_of(&step_play), icons::PLAY);
        assert_eq!(
            step_play.tooltip_text().as_deref(),
            Some(tr("Play the assembly").as_str())
        );
        assert_eq!(
            swing_play.tooltip_text().as_deref(),
            Some(tr("Play the swing").as_str())
        );
        step.show_playing(true);
        swing.show_playing(true);
        assert_eq!(shape_of(&step_play), icons::PLAY, "queued");
        settle();
        for play in [&step_play, &swing_play] {
            assert_eq!(shape_of(play), icons::PAUSE);
            assert_eq!(play.tooltip_text().as_deref(), Some(tr("Pause").as_str()));
        }
        step.show_playing(false);
        settle();
        assert_eq!(shape_of(&step_play), icons::PLAY);
        assert_eq!(
            step_play.tooltip_text().as_deref(),
            Some(tr("Play the assembly").as_str())
        );
        assert!(heard.borrow().is_empty(), "showing never calls back");
        assert!(swing_heard.borrow().is_empty(), "showing never calls back");
    }

    #[test]
    fn the_swing_card_names_its_slider_by_message() {
        on_gtk_thread(swing_names_its_slider);
    }

    fn swing_names_its_slider() {
        let (to, _heard) = recorder::<SwingCardOutput>();
        let card = SwingCard::new(&to, |output| output);
        card.show_name("drawer", false);
        settle();
        assert_eq!(
            card.controller.model().scrub.label(),
            tr("How far it slides out")
        );
    }

    #[test]
    fn the_caption_bolds_its_first_two_words_and_is_freed() {
        on_gtk_thread(caption);
    }

    fn caption() {
        let caption = Caption::new();
        caption.show("Looking down on the hinge", "back ↑ · front ↓");
        settle();
        assert_eq!(
            caption.controller.model().markup,
            "<b>Looking down</b> on the hinge"
        );
        caption.show("Above", "");
        settle();
        assert_eq!(caption.controller.model().markup, "<b>Above</b>");
        assert_eq!(caption.controller.model().hint, "");
        let root = caption.root.downgrade();
        drop(caption);
        settle();
        assert!(root.upgrade().is_none(), "the line is freed");
    }

    #[test]
    fn pictures_are_laid_out_once_and_chosen_by_message() {
        on_gtk_thread(pictures_choose);
    }

    /// The old pictures called the window from inside a button's click;
    /// now the click is a message, and the choice reaches the window after
    /// the update.
    fn pictures_choose() {
        let chosen: Rc<RefCell<Vec<usize>>> = Rc::default();
        let log = Rc::clone(&chosen);
        let to = listener(move |output| {
            if let PicturesOutput::Chosen(index) = output {
                log.borrow_mut().push(index);
            }
        });
        let pictures = Pictures::new(&to, |output| output);
        let captions = vec!["0°".to_owned(), "45°".to_owned(), "110°".to_owned()];
        pictures.show(&captions);
        assert!(pictures.root.first_child().is_none(), "show is queued");
        settle();
        let buttons: Vec<gtk::Widget> =
            std::iter::successors(pictures.root.first_child(), gtk::Widget::next_sibling).collect();
        assert_eq!(buttons.len(), 3);
        // The same captions keep the same pictures.
        pictures.show(&captions);
        settle();
        assert_eq!(pictures.root.first_child().as_ref(), buttons.first());
        pictures.show_current(Some(1));
        settle();
        let rung: Vec<bool> = buttons
            .iter()
            .map(|button| button.has_css_class("current"))
            .collect();
        assert_eq!(rung, vec![false, true, false]);
        assert_eq!(pictures.size(), None, "no room yet");
        let Some(button) = buttons
            .get(2)
            .and_then(|button| button.downcast_ref::<gtk::Button>())
        else {
            panic!("each picture is a button");
        };
        button.emit_clicked();
        assert!(chosen.borrow().is_empty(), "the click is only queued");
        settle();
        assert_eq!(*chosen.borrow(), vec![2], "the owner hears it");
    }

    /// A size in device pixels, as the pictures publish it.
    type DeviceSize = (u32, u32);

    #[test]
    fn a_resize_heard_from_inside_its_callback_is_queued() {
        on_gtk_thread(pictures_resize);
    }

    /// The old pictures' resize handler ran inside the slot's allocation;
    /// now it is a message carrying the size, reported once per size,
    /// and a size said from inside the owner's callback is queued.
    fn pictures_resize() {
        let shared: Rc<RefCell<Option<std::rc::Weak<Pictures>>>> = Rc::default();
        let heard: Rc<RefCell<Vec<Option<DeviceSize>>>> = Rc::default();
        let (log, weak) = (Rc::clone(&heard), Rc::clone(&shared));
        let to = listener(move |output| {
            let PicturesOutput::Resized(size) = output else {
                return;
            };
            log.borrow_mut().push(size);
            if log.borrow().len() == 1
                && let Some(pictures) = weak.borrow().as_ref().and_then(std::rc::Weak::upgrade)
            {
                pictures.send(PicturesInput::SlotResized {
                    width: 640,
                    height: 360,
                });
            }
        });
        let pictures = Rc::new(Pictures::new(&to, |output| output));
        shared.replace(Some(Rc::downgrade(&pictures)));
        pictures.root.set_visible(true);
        pictures.show(&["closed".to_owned()]);
        settle();
        // The slot has had no room yet: nothing to say.
        pictures.send(PicturesInput::SlotResized {
            width: 1,
            height: 1,
        });
        settle();
        assert!(heard.borrow().is_empty(), "no room, no size");
        // Room, as a parent's layout would give it.
        let _ = pictures.root.measure(gtk::Orientation::Horizontal, -1);
        pictures.root.allocate(400, 200, -1, None);
        assert!(heard.borrow().is_empty(), "queued");
        settle();
        {
            let heard = heard.borrow();
            assert!(
                heard.first().is_some_and(Option::is_some),
                "the room is said: {heard:?}"
            );
            assert_eq!(
                heard.get(1),
                Some(&Some((640, 360))),
                "the size said from inside the callback came after"
            );
        }
        for _ in 0..3 {
            pictures.send(PicturesInput::SlotResized {
                width: 640,
                height: 360,
            });
        }
        settle();
        assert_eq!(
            heard.borrow().len(),
            2,
            "once for a size, however often it is said"
        );
        // New captions lay the slots out afresh: no room until they have it.
        pictures.show(&["closed".to_owned(), "open".to_owned()]);
        settle();
        assert_eq!(heard.borrow().get(2), Some(&None), "laid out afresh");
    }

    #[test]
    fn dropped_pictures_are_freed() {
        on_gtk_thread(pictures_freed);
    }

    fn pictures_freed() {
        let (held, callback) = token();
        let to = listener(move |_: PicturesOutput| {
            let _ = &held;
        });
        let pictures = Pictures::new(&to, |output| output);
        drop(to);
        pictures.show(&["closed".to_owned(), "open".to_owned()]);
        settle();
        let root = pictures.root.downgrade();
        let slot = pictures
            .controller
            .model()
            .slots
            .first()
            .map(|(slot, _)| slot.downgrade());
        let input = pictures.controller.sender().clone();
        drop(pictures);
        settle();
        assert!(root.upgrade().is_none(), "the row is freed");
        assert!(
            slot.and_then(|slot| slot.upgrade()).is_none(),
            "its pictures are freed"
        );
        assert!(input.send(PicturesInput::ShowCurrent(None)).is_err());
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    /// A door that first touches at about 30° of 110°.
    fn door() -> MotionCard {
        let Some(assembly) = AssemblyId::from_index(0) else {
            panic!("the first assembly has an id");
        };
        MotionCard {
            assembly,
            name: "door".to_owned(),
            travel: MotionTravel::Swing {
                full: Angle::from_deg(110),
                contact: Angle::from_deg(30),
                shown: Angle::from_deg(31),
            },
            start: "closed 0°".to_owned(),
            blocked: "blocked from 30°".to_owned(),
            end: "opens to 110°".to_owned(),
        }
    }

    #[test]
    fn the_problem_card_shows_a_motion_without_calling_back() {
        on_gtk_thread(problem_card_shows);
    }

    fn problem_card_shows() {
        let (to, heard) = recorder();
        let card = ProblemCard::new(&to, |output| output);
        let door = door();
        card.show(Some(&door), 10_f32.to_radians());
        assert!(!card.root.is_visible(), "queued");
        settle();
        assert!(card.root.is_visible());
        {
            let model = card.controller.model();
            assert!(model.lead.contains("<b>Swing door</b>"));
            assert_eq!(model.value, "10°");
            assert!(!model.past);
            assert_eq!(model.blocked, "blocked from 30°");
            assert!((model.scrub.value() - 10.0).abs() < 1e-3, "in degrees");
        }
        card.show_value(&door, 40_f32.to_radians());
        settle();
        assert!(card.controller.model().past, "red past the contact");
        card.show(None, 0.0);
        settle();
        assert!(!card.root.is_visible());
        assert!(heard.borrow().is_empty(), "setters never call back");
    }

    #[test]
    fn a_swing_of_the_problem_card_is_queued_and_the_card_is_freed() {
        on_gtk_thread(problem_card_swings);
    }

    fn problem_card_swings() {
        let (held, callback) = token();
        let heard: Rc<RefCell<Vec<f64>>> = Rc::default();
        let log = Rc::clone(&heard);
        let to = listener(move |ProblemCardOutput::Moved(value)| {
            let _ = &held;
            log.borrow_mut().push(value);
        });
        let card = ProblemCard::new(&to, |output| output);
        drop(to);
        card.show(Some(&door()), 0.0);
        settle();
        let slider = card.controller.model().scrub.root.clone();
        press(&slider, 1000.0);
        assert!(heard.borrow().is_empty(), "queued");
        settle();
        assert_eq!(heard.borrow().len(), 1);
        assert!(
            heard
                .borrow()
                .iter()
                .all(|value| (value - 110.0).abs() < 1e-3)
        );
        let root = card.root.downgrade();
        drop((card, slider));
        settle();
        assert!(root.upgrade().is_none(), "the card is freed");
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }
}
