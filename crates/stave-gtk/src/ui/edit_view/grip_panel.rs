//! The size a grip moves, as a section of the change's card in the Edit
//! panel (`docs/gui.md`, "Grips"): no card floats beside the handle. It
//! says what is moved and to what (a field where a value can be typed),
//! what the size follows, what else changes, the sizes to pick from as a
//! radio list (one row each: the radio, the name, the value), the range
//! and the rule a stop names, and holds the grip's options (⋮: let the
//! solver move it, with its range). The verdict and Apply / Cancel are
//! the change card's own. It is drawn from a [`PanelModel`] and the
//! grip's switches ([`GripOptions`]), and reports what is clicked or
//! typed; it decides nothing.
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
//! It is a relm4 component ([`GripPanelComponent`])
//! following `docs/relm4-migration.md`: only `update` changes what they
//! show, and their widgets' handlers only send messages. The window owns
//! their `Controller`s (the panel's inside a [`GripSection`]), launched
//! with their outputs forwarded to its input. The section around the
//! panel's content is its owner's: the owner shows and hides it at once,
//! while what it says is drawn when the main loop next runs.

use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::i18n::{fill_in, tr};
use stave_lang::ast::Ident;
use stave_model::ids::ParamId;

use super::super::sections;
use super::grip_text::{PanelModel, SolverSwitch, suggested_range};

/// What the panel reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum PanelAction {
    /// One of the sizes to pick from.
    Pick(ParamId),
    /// A value typed in its field, Enter pressed.
    Type(String),
    /// The grip's options (⋮) asked for its size to be made plain or
    /// solved.
    Switch {
        /// The grip whose options asked.
        index: usize,
        /// What they asked.
        ask: SwitchAsk,
    },
}

/// What a grip's options menu (⋮) asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum SwitchAsk {
    /// Its switch was turned off: write the param plain again.
    MakePlain(Ident),
    /// "Make it solved", with the range as typed.
    MakeSolved {
        /// The param.
        param: Ident,
        /// The range's low end, as typed.
        low: String,
        /// The range's high end, as typed.
        high: String,
    },
}

/// Which grip options menu (⋮) asked for a param to be made solved, to
/// close it once the range is read.
#[derive(Debug)]
pub(in crate::ui) enum SwitchMenu {
    /// The menu beside a size's field in the Sizes card, the window's own.
    Row(gtk::MenuButton),
    /// The menu in the size section: the section closes it when told.
    Panel,
}

/// The grip whose options (⋮) the panel holds, and its switches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GripOptions {
    /// The grip, in the listing.
    pub(super) index: usize,
    /// What its menu can switch.
    pub(super) switches: Vec<SolverSwitch>,
}

/// The panel's content: what it says and the size it moves. Its state is
/// the value its field last showed, so a redraw doesn't undo typing, and
/// the grip its options (⋮) were made for, so they are made again only
/// for another grip; the rest is drawn from each [`PanelModel`] it is
/// shown.
#[derive(Debug, Default)]
pub(super) struct GripPanelComponent {
    /// The value last shown in the field.
    shown_value: String,
    /// The grip whose options (⋮) it holds.
    options_for: Option<usize>,
}

/// The menu beside a grip's value or number: "Let the solver move this",
/// with the range a solved param needs, or the switch to make it plain.
///
/// A stateless builder of plain widgets, not a component: the size
/// section builds it into its own view (its options, ⋮), and the window
/// builds it beside the field of its Sizes card's size editor. Its
/// handlers touch only the menu's own widgets, made here: the switch
/// shows or hides the range under it and, switched off on a solved
/// param, pops the menu down; "Make it solved" reads the two range fields
/// beside it. Each then hands what was asked, with the menu button (held
/// weakly until then), to `asked`, which every caller makes send one
/// message and do nothing else (`GripPanelInput::Switched`, or the
/// window's `EditingInput` with [`SwitchMenu::Row`]). No Rust state is
/// shared.
pub(super) fn switch_menu(
    switches: &[SolverSwitch],
    asked: impl Fn(SwitchAsk, gtk::MenuButton) + Clone + 'static,
) -> gtk::MenuButton {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(10)
        .css_classes(["stave-grip-switches"])
        .build();
    let menu = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .css_classes(["flat"])
        .tooltip_text(tr("Let the solver move this"))
        .build();
    menu.update_property(&[gtk::accessible::Property::Label(&tr(
        "Let the solver move this",
    ))]);
    let popover = gtk::Popover::builder().child(&content).build();
    menu.set_popover(Some(&popover));
    let only = switches.len() == 1;
    for switch in switches {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let template = if only {
            tr("Let the solver move this ({param})")
        } else {
            tr("Let the solver move {param}")
        };
        let words = fill_in(&template, &[("param", &switch.param.as_str())]);
        row.append(
            &gtk::Label::builder()
                .label(&words)
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        let toggle = gtk::Switch::builder()
            .active(switch.solved)
            .valign(gtk::Align::Center)
            .build();
        toggle.update_property(&[gtk::accessible::Property::Label(&words)]);
        row.append(&toggle);
        content.append(&row);
        // The range a solved param needs, typed before it is made one.
        let (low_text, high_text) = suggested_range(switch.value);
        let range = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .visible(false)
            .build();
        range.append(
            &gtk::Label::builder()
                .label(tr("The solver keeps it within:"))
                .xalign(0.0)
                .css_classes(["stave-grip-note"])
                .build(),
        );
        let ends = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let low = gtk::Entry::builder().text(&low_text).width_chars(8).build();
        let high = gtk::Entry::builder()
            .text(&high_text)
            .width_chars(8)
            .build();
        low.update_property(&[gtk::accessible::Property::Label(&tr("Lowest value"))]);
        high.update_property(&[gtk::accessible::Property::Label(&tr("Highest value"))]);
        ends.append(&low);
        ends.append(&gtk::Label::new(Some(&tr("to"))));
        ends.append(&high);
        range.append(&ends);
        let make = gtk::Button::builder()
            .label(tr("Make it solved"))
            .css_classes(["suggested-action"])
            .halign(gtk::Align::End)
            .build();
        range.append(&make);
        content.append(&range);
        let solved = switch.solved;
        let param = switch.param.clone();
        let plain = asked.clone();
        let shown = range.clone();
        // The switch and the button live inside the menu's popover, so
        // their handlers hold the menu weakly: a strong hold would be a
        // cycle that keeps every menu alive after it is replaced.
        let weak_menu = menu.downgrade();
        toggle.connect_state_set(move |_, on| {
            shown.set_visible(on && !solved);
            if !on
                && solved
                && let Some(menu) = weak_menu.upgrade()
            {
                menu.popdown();
                plain(SwitchAsk::MakePlain(param.clone()), menu);
            }
            glib::Propagation::Proceed
        });
        let param = switch.param.clone();
        let asked = asked.clone();
        let weak_menu = menu.downgrade();
        make.connect_clicked(move |_| {
            let Some(menu) = weak_menu.upgrade() else {
                return;
            };
            asked(
                SwitchAsk::MakeSolved {
                    param: param.clone(),
                    low: low.text().trim().to_owned(),
                    high: high.text().trim().to_owned(),
                },
                menu,
            );
        });
    }
    menu
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
    /// Holds the grip's options (⋮).
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
    /// Draw this (boxed: it is large beside what people do), with the
    /// options (⋮) of this grip, if it offers any; they are made again
    /// only for another grip.
    Show(Box<PanelModel>, Option<GripOptions>),
    /// Close the options' menu: what it asked was read.
    CloseOptions,
    /// People pressed Enter in the value's field with this in it.
    Typed(String),
    /// People picked this size from the radios.
    Picked(ParamId),
    /// People asked something of the options (⋮) of grip `index`.
    Switched {
        /// The grip the options were made for.
        index: usize,
        /// What they asked.
        ask: SwitchAsk,
    },
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
            GripPanelInput::Show(model, options) => {
                self.draw(widgets, &model, &sender);
                self.show_options(widgets, options, &sender);
                None
            }
            GripPanelInput::CloseOptions => {
                if let Some(menu) = widgets
                    .options
                    .first_child()
                    .and_downcast::<gtk::MenuButton>()
                {
                    menu.popdown();
                }
                None
            }
            GripPanelInput::Typed(text) => Some(PanelAction::Type(text)),
            GripPanelInput::Picked(param) => Some(PanelAction::Pick(param)),
            GripPanelInput::Switched { index, ask } => Some(PanelAction::Switch { index, ask }),
        };
        if let Some(action) = action {
            // Nobody listening is not a mistake.
            let _ = sender.output(GripPanelOutput::Action(action));
        }
    }
}

impl GripPanelComponent {
    /// Holds the options (⋮) of the grip `options` names, made again only
    /// for another grip.
    fn show_options(
        &mut self,
        widgets: &GripPanelWidgets,
        options: Option<GripOptions>,
        sender: &ComponentSender<Self>,
    ) {
        let wanted = options.as_ref().map(|options| options.index);
        if std::mem::replace(&mut self.options_for, wanted) == wanted {
            return;
        }
        clear(&widgets.options);
        if let Some(GripOptions { index, switches }) = options {
            let input = sender.input_sender().clone();
            let menu = switch_menu(&switches, move |ask, _| {
                input.emit(GripPanelInput::Switched { index, ask });
            });
            menu.set_tooltip_text(Some(&tr(
                "Options: let the solver move this, and its range",
            )));
            widgets.options.append(&menu);
        }
    }

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

/// The size section as its owner holds it: the section around the
/// panel's content (shown and hidden by the owner, at once) and the
/// component, whose outputs were forwarded to the owner when it was
/// launched. Dropping it shuts the component down.
pub(super) struct GripSection {
    /// The section, put into the change card's grip slot.
    pub(super) root: gtk::Box,
    /// The panel's content.
    pub(super) panel: Controller<GripPanelComponent>,
}

impl GripSection {
    /// The section, hidden, its actions going to `forward`'s answer on
    /// `input`.
    pub(super) fn new<Message: 'static>(
        input: &relm4::Sender<Message>,
        forward: impl Fn(GripPanelOutput) -> Message + 'static,
    ) -> GripSection {
        let (root, body) = sections::section(&tr("Size"), "", true);
        root.set_visible(false);
        let panel = GripPanelComponent::builder()
            .launch(GripPanelWidgets::new())
            .forward(input, forward);
        body.append(panel.widget());
        GripSection { root, panel }
    }

    /// Draws `model`, with the options (⋮) of the grip `options` names,
    /// when the main loop next runs.
    pub(super) fn show(&self, model: &PanelModel, options: Option<GripOptions>) {
        // After shutdown there is nothing to draw.
        let _ = self
            .panel
            .sender()
            .send(GripPanelInput::Show(Box::new(model.clone()), options));
    }

    /// Closes the options' menu once what it asked has been read.
    pub(super) fn close_options(&self) {
        let _ = self.panel.sender().send(GripPanelInput::CloseOptions);
    }
}

/// What a grip panel says, top to bottom, once what it was told is drawn,
/// for a test.
#[cfg(test)]
pub(super) fn panel_words(panel: &Controller<GripPanelComponent>) -> Vec<String> {
    crate::ui::gtk_test::settle();
    let widgets = panel.widgets();
    vec![
        widgets.note.label().to_string(),
        widgets.value.text().to_string(),
        widgets.was.label().to_string(),
        widgets.about.label().to_string(),
        widgets.stop.label().to_string(),
    ]
}

/// A grip panel's radio rows' names and whether each is picked, once
/// what it was told is drawn, for a test.
#[cfg(test)]
pub(super) fn panel_radios(panel: &Controller<GripPanelComponent>) -> Vec<(String, bool)> {
    crate::ui::gtk_test::settle();
    radio_buttons(&panel.widgets().choices)
        .iter()
        .map(|radio| {
            let name = radio
                .child()
                .and_then(|row| row.first_child())
                .and_downcast::<gtk::Label>()
                .map(|label| label.label().to_string())
                .unwrap_or_default();
            (name, radio.is_active())
        })
        .collect()
}

/// The radios in `choices`, to press them as people would, for a test.
#[cfg(test)]
fn radio_buttons(choices: &gtk::Box) -> Vec<gtk::CheckButton> {
    let mut radios = Vec::new();
    let mut child = choices.first_child();
    while let Some(widget) = child {
        if let Ok(radio) = widget.clone().downcast::<gtk::CheckButton>() {
            radios.push(radio);
        }
        child = widget.next_sibling();
    }
    radios
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// The sizes' own few colours (libadwaita's named colours), installed
/// once per display: the Size section, the Sizes card and its number
/// fields.
const STYLE: &str = r#"
.stave-grip-title { font-family: var(--monospace-font-family, monospace); }
entry.stave-grip-value { font-family: var(--monospace-font-family, monospace); }
.stave-grip-change { font-size: 0.818rem; font-family: var(--monospace-font-family, monospace); color: #99c1f1; }
.stave-grip-ask { font-size: 0.818rem; font-weight: 700; margin-top: 4px; }
.stave-grip-stop { color: @error_color; }
.stave-grip-switches { padding: 8px; }
.stave-grip-switches .stave-grip-note { font-size: 0.818rem; opacity: 0.75; }
"#;

/// Puts the grips' stylesheet on the display. The application calls it
/// once, at startup, beside the window's own style (`style::install`);
/// tests do it once for the GTK test thread (`gtk_test::on_gtk_thread`).
pub(in crate::ui) fn install_style(display: &gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
    let sizes = gtk::CssProvider::new();
    sizes.load_from_string(super::sizes::STYLE);
    gtk::style_context_add_provider_for_display(
        display,
        &sizes,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
    );
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
        }
    }

    /// A section, and what it reports as its owner hears it: from the
    /// main loop, after the panel's update.
    fn heard_section() -> (GripSection, Rc<RefCell<Vec<PanelAction>>>) {
        let (sender, receiver) = relm4::channel::<PanelAction>();
        let section = GripSection::new(&sender, |output| match output {
            GripPanelOutput::Action(action) => action,
        });
        let heard: Rc<RefCell<Vec<PanelAction>>> = Rc::default();
        let log = Rc::clone(&heard);
        relm4::spawn_local(async move {
            while let Some(action) = receiver.recv().await {
                log.borrow_mut().push(action);
            }
        });
        (section, heard)
    }

    #[test]
    fn a_dropped_grip_panel_is_freed() {
        on_gtk_thread(grip_panel_is_freed);
    }

    /// Dropping the section drops its controller: the content, the model
    /// and the forwarding are freed once the main loop runs.
    fn grip_panel_is_freed() {
        let (sender, receiver) = relm4::channel::<PanelAction>();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let section = GripSection::new(&sender, move |output| {
            let _ = &held;
            match output {
                GripPanelOutput::Action(action) => action,
            }
        });
        let forwarding = Rc::downgrade(&token);
        drop(token);
        drop(sender);
        section.show(&two_choices(), None);
        settle();
        let content = section.panel.widget().downgrade();
        let root = section.root.downgrade();
        let input = section.panel.sender().clone();
        drop(section);
        settle();
        assert!(content.upgrade().is_none(), "the content is freed");
        assert!(root.upgrade().is_none(), "the section is freed");
        assert!(
            input.send(GripPanelInput::Typed("1mm".to_owned())).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(forwarding.upgrade().is_none(), "the forwarding is freed");
        drop(receiver);
    }

    #[test]
    fn showing_the_grip_panel_reports_nothing() {
        on_gtk_thread(showing_reports_nothing);
    }

    fn showing_reports_nothing() {
        let (section, heard) = heard_section();
        section.show(&two_choices(), None);
        section.show(&two_choices(), None);
        assert_eq!(
            panel_radios(&section.panel),
            [
                ("overall_width".to_owned(), true),
                ("case.overhang".to_owned(), false)
            ]
        );
        assert!(heard.borrow().is_empty(), "drawing it picks nothing");
    }

    #[test]
    fn typing_a_value_and_pressing_enter_reports_it() {
        on_gtk_thread(typing_is_reported);
    }

    fn typing_is_reported() {
        let (section, heard) = heard_section();
        section.show(&two_choices(), None);
        settle();
        let entry = section.panel.widgets().value.clone();
        entry.set_text(" 1200mm ");
        entry.emit_activate();
        assert!(heard.borrow().is_empty(), "only queued");
        settle();
        assert_eq!(*heard.borrow(), [PanelAction::Type("1200mm".to_owned())]);
    }

    #[test]
    fn a_pick_made_while_a_pick_is_handled_is_queued() {
        on_gtk_thread(picks_are_queued);
    }

    /// A pick only queues a message; the action reaches the owner after
    /// the panel's update has returned. The owner's handler redraws the
    /// panel and presses another radio: both are queued, not run under
    /// it.
    fn picks_are_queued() {
        let heard: Rc<RefCell<Vec<PanelAction>>> = Rc::default();
        let log = Rc::clone(&heard);
        let (sender, receiver) = relm4::channel::<PanelAction>();
        let section = GripSection::new(&sender, |output| match output {
            GripPanelOutput::Action(action) => action,
        });
        section.show(&two_choices(), None);
        settle();
        let input = section.panel.sender().clone();
        let choices = section.panel.widgets().choices.clone();
        // The owner: hears each action from the main loop.
        relm4::spawn_local(async move {
            while let Some(action) = receiver.recv().await {
                let first = log.borrow().is_empty();
                log.borrow_mut().push(action);
                if first {
                    let _ = input.send(GripPanelInput::Show(Box::new(two_choices()), None));
                    // The radios the queued redraw has not replaced yet.
                    if let Some(radio) = radio_buttons(&choices).first() {
                        radio.set_active(true);
                    }
                    assert_eq!(log.borrow().len(), 1, "nothing ran under the handler");
                }
            }
        });
        let radios = radio_buttons(&section.panel.widgets().choices);
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
        drop(sender);
        drop(section);
        settle();
    }

    /// The options' menu the section holds, if any.
    fn options_menu(section: &GripSection) -> Option<gtk::MenuButton> {
        section
            .panel
            .widgets()
            .options
            .first_child()
            .and_downcast::<gtk::MenuButton>()
    }

    #[test]
    fn the_grips_options_are_the_panels_own_and_report_by_message() {
        on_gtk_thread(options_are_the_panels);
    }

    /// The window used to put the grip's options (⋮) into the panel
    /// itself; now they come with `Show`, are made again only for another
    /// grip, and what they ask reaches the owner as an action.
    fn options_are_the_panels() {
        let (section, heard) = heard_section();
        let width = Ident::new("width", stave_lang::ast::Span::default()).unwrap();
        let options = |index: usize| GripOptions {
            index,
            switches: vec![SolverSwitch {
                param: width.clone(),
                value: super::super::grip_text::mm_amount(100.0).unwrap(),
                solved: false,
            }],
        };
        section.show(&two_choices(), Some(options(3)));
        assert!(options_menu(&section).is_none(), "only queued");
        settle();
        let Some(menu) = options_menu(&section) else {
            panic!("the grip's options are shown");
        };
        section.show(&two_choices(), Some(options(3)));
        settle();
        assert_eq!(
            options_menu(&section).as_ref(),
            Some(&menu),
            "kept for the same grip"
        );
        section.show(&two_choices(), Some(options(4)));
        settle();
        let Some(other) = options_menu(&section) else {
            panic!("another grip's options are shown");
        };
        assert_ne!(other, menu, "made again for another grip");
        // "Make it solved", as people would press it.
        let Some(content) = other.popover().and_then(|popover| popover.child()) else {
            panic!("the menu has its content");
        };
        let make = std::iter::successors(Some(content.upcast::<gtk::Widget>()), |widget| {
            widget
                .first_child()
                .or_else(|| widget.next_sibling())
                .or_else(|| {
                    std::iter::successors(widget.parent(), gtk::Widget::parent)
                        .find_map(|parent| parent.next_sibling())
                })
        })
        .find_map(|widget| widget.downcast::<gtk::Button>().ok());
        let Some(make) = make else {
            panic!("the menu has its Make it solved button");
        };
        make.emit_clicked();
        assert!(heard.borrow().is_empty(), "only queued");
        settle();
        let Some(PanelAction::Switch {
            index: 4,
            ask: SwitchAsk::MakeSolved { param, .. },
        }) = heard.borrow().first().cloned()
        else {
            panic!("the owner hears what was asked: {:?}", heard.borrow());
        };
        assert_eq!(param, width);
        section.close_options();
        settle();
        section.show(&two_choices(), None);
        settle();
        assert!(options_menu(&section).is_none(), "no options, no menu");
    }
}
