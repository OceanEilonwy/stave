//! The size a grip moves, as a section of the change's card in the Edit
//! panel (`docs/gui.md`, "Grips"): no card floats beside the handle. It
//! says what is moved and to what (a field where a value can be typed),
//! what the size follows, what else changes, the sizes to pick from as a
//! radio list (one row each: the radio, the name, the value), the range
//! and the rule a stop names, and holds the grip's options (⋮: let the
//! solver move it, with its range). The verdict and Apply / Cancel are
//! the change card's own. It is drawn from a [`PanelModel`] and reports
//! what is clicked or typed; it decides nothing.
//!
//! ```text
//! ▾ Size
//!   Grip         case.base.length
//!   Value        [1000mm    ] was 900mm  ⋮
//!   Follows      overall_width
//!   Which size changes?
//!   (•) overall_width             940 → 1040 mm
//!   ( ) case.overhang              20 → −30 mm
//!   Stops        stops at 1200mm, because …
//! ```
//!
//! Both are relm4 components ([`GripPanelComponent`], [`ReadoutComponent`])
//! following `docs/relm4-migration.md`: only `update` changes what they
//! show, and their widgets' handlers only send messages. [`GripPanel`] and
//! [`Readout`] keep the older API in front of them while the grips'
//! coordinator (`grips.rs`, `impl Window`) is still old code. The section
//! around the panel's content is its owner's: the owner shows and hides it
//! at once, while what it says is drawn when the main loop next runs.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::gdk;
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::i18n::{fill_in, tr};
use stave_model::ids::ParamId;

use super::super::relm4_shim::Bridge;
use super::super::sections;
use super::grip_text::PanelModel;

/// What the panel reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum PanelAction {
    /// One of the sizes to pick from.
    Pick(ParamId),
    /// A value typed in its field, Enter pressed.
    Type(String),
}

/// The panel's content: what it says and the size it moves. Its only
/// state is the value its field last showed, so a redraw doesn't undo
/// typing; the rest is drawn from each [`PanelModel`] it is shown.
#[derive(Debug, Default)]
pub(super) struct GripPanelComponent {
    /// The value last shown in the field.
    shown_value: String,
}

/// The panel's widgets: the rows of its content, made by
/// [`GripPanelWidgets::new`] and handed to the component at launch.
#[derive(Debug, Clone)]
pub(super) struct GripPanelWidgets {
    /// What is moved: the grip.
    note: gtk::Label,
    /// The value, typed over.
    value: gtk::Entry,
    /// "was 900mm", after the field.
    was: gtk::Label,
    /// Where the window puts the grip's options (⋮).
    options: gtk::Box,
    /// The row under the value: "Follows" or "Note".
    about_row: gtk::Box,
    about_label: gtk::Label,
    about: gtk::Label,
    /// What else changes, a line each.
    changes: gtk::Box,
    /// The sizes to pick from, as radios.
    choices: gtk::Box,
    /// The rule a stop names.
    stop_row: gtk::Box,
    stop: gtk::Label,
    /// What to do next, while the change waits.
    waiting: gtk::Label,
}

impl GripPanelWidgets {
    /// The rows, empty.
    fn new() -> GripPanelWidgets {
        let note = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .css_classes(["stave-field-value", "stave-grip-title"])
            .build();
        let value = gtk::Entry::builder()
            .hexpand(true)
            .width_chars(6)
            .css_classes(["stave-grip-value"])
            .tooltip_text(tr("Type a value and press Enter"))
            .build();
        value.update_property(&[gtk::accessible::Property::Label(&tr("Value"))]);
        let options = gtk::Box::builder().valign(gtk::Align::Center).build();
        let was = gtk::Label::builder()
            .css_classes(["stave-row-note"])
            .build();
        let about = sections::value("");
        about.set_hexpand(true);
        let about_label = sections::row_label(&tr("Follows"));
        let about_row = gtk::Box::builder()
            .spacing(6)
            .css_classes(["stave-field-row"])
            .build();
        let changes = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let choices = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .accessible_role(gtk::AccessibleRole::RadioGroup)
            .build();
        choices.update_property(&[gtk::accessible::Property::Label(&tr("Which size changes?"))]);
        let stop = sections::value("");
        stop.add_css_class("stave-grip-stop");
        let stop_row = sections::row(&tr("Stops"), &stop);
        let waiting = sections::words("", &["stave-row-note"]);
        GripPanelWidgets {
            note,
            value,
            was,
            options,
            about_row,
            about_label,
            about,
            changes,
            choices,
            stop_row,
            stop,
            waiting,
        }
    }
}

/// What the panel is told.
#[derive(Debug)]
pub(super) enum GripPanelInput {
    /// Draw this (boxed: it is large beside what people do).
    Show(Box<PanelModel>),
    /// People pressed Enter in the value's field with this in it.
    Typed(String),
    /// People picked this size from the radios.
    Picked(ParamId),
}

/// What the panel tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GripPanelOutput {
    /// Something was clicked or typed.
    Action(PanelAction),
}

impl Component for GripPanelComponent {
    type Init = GripPanelWidgets;
    type Input = GripPanelInput;
    type Output = GripPanelOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = GripPanelWidgets;

    fn init_root() -> gtk::Box {
        gtk::Box::new(gtk::Orientation::Vertical, 0)
    }

    fn init(
        widgets: GripPanelWidgets,
        root: gtk::Box,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        root.append(&sections::row(&tr("Grip"), &widgets.note));
        let value_line = gtk::Box::builder().spacing(4).hexpand(true).build();
        value_line.append(&widgets.value);
        value_line.append(&widgets.was);
        value_line.append(&widgets.options);
        root.append(&sections::row(&tr("Value"), &value_line));
        widgets.about_row.append(&widgets.about_label);
        widgets.about_row.append(&widgets.about);
        root.append(&widgets.about_row);
        root.append(&widgets.changes);
        root.append(&widgets.choices);
        root.append(&widgets.stop_row);
        root.append(&widgets.waiting);
        // Enter in the field sends what is typed in it.
        let input = sender.input_sender().clone();
        widgets.value.connect_activate(move |entry| {
            let text = entry.text().trim().to_owned();
            if !text.is_empty() {
                input.emit(GripPanelInput::Typed(text));
            }
        });
        ComponentParts {
            model: GripPanelComponent::default(),
            widgets,
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut GripPanelWidgets,
        message: GripPanelInput,
        sender: ComponentSender<Self>,
        _root: &gtk::Box,
    ) {
        let action = match message {
            GripPanelInput::Show(model) => {
                self.draw(widgets, &model, &sender);
                None
            }
            GripPanelInput::Typed(text) => Some(PanelAction::Type(text)),
            GripPanelInput::Picked(param) => Some(PanelAction::Pick(param)),
        };
        if let Some(action) = action {
            // Nobody listening is not a mistake.
            let _ = sender.output(GripPanelOutput::Action(action));
        }
    }
}

impl GripPanelComponent {
    /// Draws `model` on the widgets.
    fn draw(
        &mut self,
        widgets: &GripPanelWidgets,
        model: &PanelModel,
        sender: &ComponentSender<Self>,
    ) {
        widgets.note.set_label(&model.title);
        // The value in the field (so it can be typed over), "was 900mm"
        // after it.
        let value = model.value.clone().unwrap_or_default();
        let was = model.was.as_ref().map_or_else(String::new, |was| {
            fill_in(&tr("was {value}"), &[("value", was)])
        });
        // The field keeps what is being typed in it.
        if self.shown_value != value && !widgets.value.has_focus() {
            widgets.value.set_text(&value);
        }
        self.shown_value = value;
        widgets.was.set_label(&was);
        widgets.was.set_visible(!was.is_empty());
        // "overall_width" under "Follows"; anything else ("solved",
        // "stuck: …") under "Note".
        match &model.about {
            Some(about) => {
                widgets.about_label.set_label(&about.label);
                widgets.about.set_label(&about.words);
                widgets.about_row.set_visible(true);
            }
            None => widgets.about_row.set_visible(false),
        }
        clear(&widgets.changes);
        for line in &model.changes {
            widgets
                .changes
                .append(&sections::words(line, &["stave-grip-change"]));
        }
        widgets.changes.set_visible(!model.changes.is_empty());
        clear(&widgets.choices);
        if !model.choices.is_empty() {
            widgets.choices.append(&sections::words(
                &tr("Which size changes?"),
                &["stave-grip-ask"],
            ));
            let mut group: Option<gtk::CheckButton> = None;
            for choice in &model.choices {
                let row = gtk::Box::builder()
                    .spacing(6)
                    .css_classes(["stave-radio-row"])
                    .build();
                row.append(
                    &gtk::Label::builder()
                        .label(&choice.name)
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk::pango::EllipsizeMode::End)
                        .css_classes(["stave-radio-name"])
                        .build(),
                );
                row.append(
                    &gtk::Label::builder()
                        .label(&choice.value)
                        .xalign(1.0)
                        .ellipsize(gtk::pango::EllipsizeMode::Start)
                        .css_classes(["stave-radio-value"])
                        .build(),
                );
                let radio = gtk::CheckButton::builder()
                    .child(&row)
                    .active(choice.picked)
                    .sensitive(choice.possible)
                    .tooltip_text(&choice.words)
                    .build();
                radio.update_property(&[gtk::accessible::Property::Label(&choice.words)]);
                if let Some(first) = &group {
                    radio.set_group(Some(first));
                } else {
                    group = Some(radio.clone());
                }
                // Connected once it is filled in, so only people's picks
                // are heard.
                let input = sender.input_sender().clone();
                let param = choice.param;
                radio.connect_toggled(move |radio| {
                    if radio.is_active() {
                        input.emit(GripPanelInput::Picked(param));
                    }
                });
                widgets.choices.append(&radio);
            }
        }
        widgets.choices.set_visible(!model.choices.is_empty());
        widgets
            .stop
            .set_label(model.stop.as_deref().unwrap_or_default());
        widgets.stop_row.set_visible(model.stop.is_some());
        // The verdict is the card's; what to do next is said here.
        let waiting = (model.verdict.class == "waiting").then_some(&model.verdict.words);
        widgets
            .waiting
            .set_label(waiting.map_or("", String::as_str));
        widgets.waiting.set_visible(waiting.is_some());
    }
}

/// The size section, behind the callback API the grips still use: it owns
/// the component (dropping it shuts the component down), passes
/// [`GripPanel::show`] on as a message and the component's actions to the
/// callback.
pub(super) struct GripPanel {
    /// The section, put into the change card's grip slot; its owner shows
    /// and hides it.
    pub(super) root: gtk::Box,
    /// Where the window puts the grip's options (⋮).
    pub(super) options: gtk::Box,
    bridge: Bridge<GripPanelComponent>,
    /// The widgets, read by tests.
    #[cfg(test)]
    widgets: GripPanelWidgets,
}

impl GripPanel {
    /// The section, hidden.
    pub(super) fn new() -> GripPanel {
        let (root, body) = sections::section(&tr("Size"), "", true);
        root.set_visible(false);
        let widgets = GripPanelWidgets::new();
        let options = widgets.options.clone();
        #[cfg(test)]
        let kept = widgets.clone();
        let bridge = Bridge::<GripPanelComponent>::launch(widgets);
        body.append(bridge.widget());
        GripPanel {
            root,
            options,
            bridge,
            #[cfg(test)]
            widgets: kept,
        }
    }

    /// Calls `callback` with what is clicked or typed.
    pub(super) fn connect_action(&self, callback: impl Fn(PanelAction) + 'static) {
        self.bridge.connect(move |output| match output {
            GripPanelOutput::Action(action) => callback(action),
        });
    }

    /// Draws `model` when the main loop next runs.
    pub(super) fn show(&self, model: &PanelModel) {
        self.bridge
            .send(GripPanelInput::Show(Box::new(model.clone())));
    }

    /// What the section says, top to bottom, once what it was told is
    /// drawn, for a test.
    #[cfg(test)]
    pub(super) fn words(&self) -> Vec<String> {
        crate::ui::gtk_test::settle();
        let widgets = &self.widgets;
        vec![
            widgets.note.label().to_string(),
            widgets.value.text().to_string(),
            widgets.was.label().to_string(),
            widgets.about.label().to_string(),
            widgets.stop.label().to_string(),
        ]
    }

    /// The radio rows' names and whether each is picked, once what it was
    /// told is drawn, for a test.
    #[cfg(test)]
    pub(super) fn radios(&self) -> Vec<(String, bool)> {
        crate::ui::gtk_test::settle();
        let mut radios = Vec::new();
        let mut child = self.widgets.choices.first_child();
        while let Some(widget) = child {
            if let Some(radio) = widget.downcast_ref::<gtk::CheckButton>() {
                let name = radio
                    .child()
                    .and_then(|row| row.first_child())
                    .and_downcast::<gtk::Label>()
                    .map(|label| label.label().to_string())
                    .unwrap_or_default();
                radios.push((name, radio.is_active()));
            }
            child = widget.next_sibling();
        }
        radios
    }

    /// The radios, to press them as people would, for a test.
    #[cfg(test)]
    fn radio_buttons(&self) -> Vec<gtk::CheckButton> {
        let mut radios = Vec::new();
        let mut child = self.widgets.choices.first_child();
        while let Some(widget) = child {
            if let Ok(radio) = widget.clone().downcast::<gtk::CheckButton>() {
                radios.push(radio);
            }
            child = widget.next_sibling();
        }
        radios
    }
}

/// The one-line readout at the top of the 3D view while a grip is moved:
/// `overall_width 940 → 1040 mm`, the name in light blue. Its model is the
/// line it shows, or `None` while it is hidden.
#[derive(Debug, Default)]
pub(super) struct ReadoutComponent {
    line: Option<(String, String)>,
}

/// What the readout is told.
#[derive(Debug)]
pub(super) enum ReadoutInput {
    /// Show this name and value, or hide it for `None`.
    Show(Option<(String, String)>),
}

#[relm4::component(pub(super))]
impl Component for ReadoutComponent {
    type Init = ();
    type Input = ReadoutInput;
    type Output = ();
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_spacing: 8,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Start,
            set_margin_top: 10,
            set_can_target: false,
            add_css_class: "stave-grip-readout",
            #[watch]
            set_visible: model.line.is_some(),

            #[name = "name"]
            gtk::Label {
                add_css_class: "stave-readout-name",
                #[watch]
                set_label: model.line.as_ref().map_or("", |(name, _)| name.as_str()),
            },
            #[name = "value"]
            gtk::Label {
                add_css_class: "stave-readout-value",
                #[watch]
                set_label: model.line.as_ref().map_or("", |(_, value)| value.as_str()),
            },
        }
    }

    fn init(_: (), root: Self::Root, _sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = ReadoutComponent::default();
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: ReadoutInput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ReadoutInput::Show(line) => self.line = line,
        }
    }
}

/// The readout, behind the API the grips still use: it owns the component
/// and passes [`Readout::show`] on as a message.
pub(super) struct Readout {
    /// The overlay.
    pub(super) root: gtk::Box,
    bridge: Bridge<ReadoutComponent>,
}

impl Readout {
    /// The readout, hidden.
    pub(super) fn new() -> Readout {
        let bridge = Bridge::<ReadoutComponent>::launch(());
        Readout {
            root: bridge.widget().clone(),
            bridge,
        }
    }

    /// Shows `name` and `value`, or hides it for `None`, when the main
    /// loop next runs.
    pub(super) fn show(&self, line: Option<&(String, String)>) {
        self.bridge.send(ReadoutInput::Show(line.cloned()));
    }

    /// What it says once what it was told is shown, for a test.
    #[cfg(test)]
    pub(super) fn words(&self) -> Option<String> {
        crate::ui::gtk_test::settle();
        self.bridge
            .model()
            .line
            .as_ref()
            .filter(|_| self.root.is_visible())
            .map(|(name, value)| format!("{name} {value}"))
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// The grips' own few colours (libadwaita's named colours), installed
/// once per display. The handles are small round dots (white, a blue
/// ring), the same for every grip: what a size follows is said in words.
const STYLE: &str = r#"
.stave-grip-title { font-family: monospace; }
entry.stave-grip-value { font-family: monospace; }
.stave-grip-change { font-size: 12px; font-family: monospace; color: #99c1f1; }
.stave-grip-ask { font-size: 12px; font-weight: 700; margin-top: 4px; }
.stave-grip-stop { color: @error_color; }
.stave-grip-handle { min-width: 8px; min-height: 8px; border-radius: 999px; background: white;
  border: 2px solid @accent_color; box-shadow: 0 1px 2px alpha(black, 0.35); }
.stave-grip-handle:focus-visible { outline: 2px solid alpha(@accent_color, 0.5); outline-offset: 2px; }
.stave-grip-handle.dragging { background: @accent_color; }
.stave-grip-lock { min-width: 18px; min-height: 18px; border-radius: 999px; background: @window_bg_color;
  color: @window_fg_color; box-shadow: 0 1px 3px alpha(black, 0.35); -gtk-icon-size: 12px; }
.stave-grip-lines { color: @accent_color; }
.stave-grip-label { padding: 0; border-radius: 6px; background: alpha(@window_bg_color, 0.92);
  color: @window_fg_color; box-shadow: 0 1px 3px alpha(black, 0.25); }
.stave-grip-label button.stave-grip-number { color: @window_fg_color; padding: 0 6px; min-height: 20px;
  font-weight: 600; font-size: 11.5px; font-family: monospace; }
.stave-grip-label.locked { padding: 2px 6px; font-size: 11.5px; opacity: 0.85; }
.stave-grip-label entry { min-height: 22px; font-size: 12px; }
.stave-grip-label .stave-grip-error { color: @error_color; font-size: 12px; font-weight: 600;
  padding: 2px 6px 4px 6px; }
.stave-grip-label menubutton > button { min-height: 20px; min-width: 20px; padding: 0 2px; }
.stave-grip-switches { padding: 8px; }
.stave-grip-switches .stave-grip-note { font-size: 12px; opacity: 0.75; }
"#;

/// Puts the grips' stylesheet on the display, once.
pub(super) fn install_style(display: &gdk::Display) {
    thread_local! {
        static INSTALLED: RefCell<Vec<gdk::Display>> = const { RefCell::new(Vec::new()) };
    }
    INSTALLED.with(|installed| {
        let mut installed = installed.borrow_mut();
        if installed.contains(display) {
            return;
        }
        let provider = gtk::CssProvider::new();
        provider.load_from_string(STYLE);
        gtk::style_context_add_provider_for_display(
            display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        installed.push(display.clone());
    });
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::edit_view::grip_text::{PanelAbout, PanelChoice, PanelVerdict};
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// A panel to pick between two sizes, the first picked.
    fn two_choices() -> PanelModel {
        let choice = |index: usize, name: &str, picked: bool| PanelChoice {
            param: ParamId::from_index(index).unwrap(),
            words: name.to_owned(),
            name: name.to_owned(),
            value: "940 → 1040 mm".to_owned(),
            possible: true,
            picked,
        };
        PanelModel {
            title: "case.base.length".to_owned(),
            value: Some("1000mm".to_owned()),
            was: Some("900mm".to_owned()),
            about: Some(PanelAbout {
                label: "Follows".to_owned(),
                words: "overall_width".to_owned(),
            }),
            changes: vec!["Changes overall_width 940mm to 1040mm".to_owned()],
            choices: vec![
                choice(0, "overall_width", true),
                choice(1, "case.overhang", false),
            ],
            stop: None,
            verdict: PanelVerdict {
                class: "waiting",
                words: "Pick the size that changes".to_owned(),
            },
            can_apply: false,
            readout: None,
        }
    }

    #[test]
    fn a_dropped_grip_panel_is_freed() {
        on_gtk_thread(grip_panel_is_freed);
    }

    /// Dropping the panel drops its controller: the content, the model
    /// and the callback are freed once the main loop runs.
    fn grip_panel_is_freed() {
        let panel = GripPanel::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        panel.connect_action(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        panel.show(&two_choices());
        settle();
        let content = panel.bridge.widget().downgrade();
        let input = panel.bridge.sender().clone();
        drop(panel);
        settle();
        assert!(content.upgrade().is_none(), "the content is freed");
        assert!(
            input.send(GripPanelInput::Typed("1mm".to_owned())).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn showing_the_grip_panel_reports_nothing() {
        on_gtk_thread(showing_reports_nothing);
    }

    fn showing_reports_nothing() {
        let panel = GripPanel::new();
        let heard: Rc<RefCell<Vec<PanelAction>>> = Rc::default();
        let log = Rc::clone(&heard);
        panel.connect_action(move |action| log.borrow_mut().push(action));
        panel.show(&two_choices());
        panel.show(&two_choices());
        assert_eq!(
            panel.radios(),
            [
                ("overall_width".to_owned(), true),
                ("case.overhang".to_owned(), false)
            ]
        );
        assert!(heard.borrow().is_empty(), "drawing it picks nothing");
    }

    #[test]
    fn a_pick_made_while_a_pick_is_handled_is_queued() {
        on_gtk_thread(picks_are_queued);
    }

    /// A pick only queues a message; the action reaches the callback after
    /// the panel's update has returned. The callback redraws the panel
    /// (which the old one did from inside the radio's own signal) and
    /// presses another radio: both are queued, not run under it.
    fn picks_are_queued() {
        let panel = Rc::new(GripPanel::new());
        panel.show(&two_choices());
        settle();
        let heard: Rc<RefCell<Vec<PanelAction>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&panel);
        panel.connect_action(move |action| {
            let Some(panel) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            log.borrow_mut().push(action);
            if first {
                panel.show(&two_choices());
                // The radios the queued redraw has not replaced yet.
                if let Some(radio) = panel.radio_buttons().first() {
                    radio.set_active(true);
                }
                assert_eq!(log.borrow().len(), 1, "nothing ran under the callback");
            }
        });
        let radios = panel.radio_buttons();
        let Some(second) = radios.get(1) else {
            panic!("two radios are drawn");
        };
        second.set_active(true);
        assert!(heard.borrow().is_empty(), "the pick is only queued");
        settle();
        let pick = |index: usize| PanelAction::Pick(ParamId::from_index(index).unwrap());
        assert_eq!(
            *heard.borrow(),
            vec![pick(1), pick(0)],
            "each pick after its update, the inner one after the first"
        );
    }

    #[test]
    fn a_dropped_readout_is_freed() {
        on_gtk_thread(readout_is_freed);
    }

    fn readout_is_freed() {
        let readout = Readout::new();
        readout.show(Some(&("overall_width".to_owned(), "940 mm".to_owned())));
        settle();
        assert!(readout.root.is_visible(), "a line shows it");
        let root = readout.root.downgrade();
        let input = readout.bridge.sender().clone();
        drop(readout);
        settle();
        assert!(root.upgrade().is_none(), "the widget is freed");
        assert!(
            input.send(ReadoutInput::Show(None)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
    }

    #[test]
    fn the_readout_shows_once_the_main_loop_runs() {
        on_gtk_thread(readout_is_queued);
    }

    fn readout_is_queued() {
        let readout = Readout::new();
        readout.show(Some(&("overall_width".to_owned(), "940 mm".to_owned())));
        assert!(!readout.root.is_visible(), "only queued");
        assert_eq!(readout.words().as_deref(), Some("overall_width 940 mm"));
        readout.show(None);
        assert_eq!(readout.words(), None);
    }
}
