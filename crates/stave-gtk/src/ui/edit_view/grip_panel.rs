//! The small card beside a grip while it is dragged, typed or switched
//! (`docs/gui-grips.md`): what is moved and to what, what else changes,
//! the sizes to pick from, the rule a stop names, the verdict in words,
//! and Apply and Cancel. It is drawn from a [`PanelModel`] and reports
//! what is clicked; it decides nothing.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use stave_model::ids::ParamId;

use super::grip_text::PanelModel;

/// What the panel reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PanelAction {
    /// One of the sizes to pick from.
    Pick(ParamId),
    /// Write it.
    Apply,
    /// Drop it.
    Cancel,
}

type ActionCallback = Rc<dyn Fn(PanelAction)>;

/// The panel.
pub(super) struct GripPanel {
    /// The widget, floated over the 3D view.
    pub(super) root: gtk::Box,
    title: gtk::Label,
    value: gtk::Label,
    changes: gtk::Box,
    choices: gtk::Box,
    stop: gtk::Box,
    stop_words: gtk::Label,
    verdict: gtk::Label,
    buttons: gtk::Box,
    apply: gtk::Button,
    callback: Rc<RefCell<Option<ActionCallback>>>,
}

impl GripPanel {
    /// The panel, hidden.
    pub(super) fn new() -> GripPanel {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .css_classes(["stave-grip-panel"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .width_request(280)
            .visible(false)
            .build();
        let title = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["stave-grip-title"])
            .wrap(true)
            .build();
        let value = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["stave-grip-value"])
            .build();
        let changes = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let choices = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let stop = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["stave-grip-stop"])
            .build();
        stop.append(
            &gtk::Image::builder()
                .icon_name("changes-prevent-symbolic")
                .valign(gtk::Align::Start)
                .build(),
        );
        let stop_words = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(34)
            .hexpand(true)
            .build();
        stop.append(&stop_words);
        let verdict = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(34)
            .css_classes(["stave-grip-verdict"])
            .build();
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .halign(gtk::Align::End)
            .margin_top(4)
            .build();
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .tooltip_text("Put it back as it was (Esc)")
            .build();
        let apply = gtk::Button::builder()
            .label("Apply")
            .css_classes(["suggested-action"])
            .build();
        buttons.append(&cancel);
        buttons.append(&apply);
        for child in [
            title.upcast_ref::<gtk::Widget>(),
            value.upcast_ref(),
            changes.upcast_ref(),
            choices.upcast_ref(),
            stop.upcast_ref(),
            verdict.upcast_ref(),
            buttons.upcast_ref(),
        ] {
            root.append(child);
        }
        let panel = GripPanel {
            root,
            title,
            value,
            changes,
            choices,
            stop,
            stop_words,
            verdict,
            buttons,
            apply,
            callback: Rc::new(RefCell::new(None)),
        };
        let callback = panel.callback.clone();
        cancel.connect_clicked(move |_| report(&callback, PanelAction::Cancel));
        let callback = panel.callback.clone();
        panel
            .apply
            .connect_clicked(move |_| report(&callback, PanelAction::Apply));
        panel
    }

    /// Calls `callback` with what is clicked.
    pub(super) fn connect_action(&self, callback: impl Fn(PanelAction) + 'static) {
        *self.callback.borrow_mut() = Some(Rc::new(callback));
    }

    /// Draws `model`. `held` hides Apply and Cancel while the pointer
    /// still holds the handle (letting go applies it when it fits).
    pub(super) fn show(&self, model: &PanelModel, held: bool) {
        self.title.set_label(&model.title);
        self.value
            .set_label(model.value.as_deref().unwrap_or_default());
        self.value.set_visible(model.value.is_some());
        clear(&self.changes);
        for line in &model.changes {
            self.changes.append(
                &gtk::Label::builder()
                    .label(line)
                    .xalign(0.0)
                    .wrap(true)
                    .max_width_chars(34)
                    .css_classes(["stave-grip-change"])
                    .build(),
            );
        }
        self.changes.set_visible(!model.changes.is_empty());
        clear(&self.choices);
        if !model.choices.is_empty() {
            self.choices.append(
                &gtk::Label::builder()
                    .label("Which size changes?")
                    .xalign(0.0)
                    .css_classes(["stave-grip-ask"])
                    .build(),
            );
            for choice in &model.choices {
                let button = gtk::ToggleButton::builder()
                    .label(&choice.words)
                    .active(choice.picked)
                    .sensitive(choice.possible)
                    .css_classes(["stave-grip-choice"])
                    .build();
                if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                    label.set_xalign(0.0);
                    label.set_wrap(true);
                }
                let callback = self.callback.clone();
                let param = choice.param;
                button.connect_clicked(move |_| report(&callback, PanelAction::Pick(param)));
                self.choices.append(&button);
            }
        }
        self.choices.set_visible(!model.choices.is_empty());
        self.stop_words
            .set_label(model.stop.as_deref().unwrap_or_default());
        self.stop.set_visible(model.stop.is_some());
        for class in ["fits", "refused", "checking", "waiting"] {
            self.verdict.remove_css_class(class);
        }
        self.verdict.add_css_class(model.verdict.class);
        self.verdict.set_label(&model.verdict.words);
        self.apply.set_sensitive(model.can_apply);
        self.apply.set_tooltip_text(Some(if model.can_apply {
            "Write it into the design (Enter)"
        } else {
            "Only a value that fits can be written"
        }));
        self.buttons.set_visible(!held);
    }

    /// What the panel says, top to bottom, for a test.
    #[cfg(test)]
    pub(super) fn words(&self) -> Vec<String> {
        [&self.title, &self.value, &self.stop_words, &self.verdict]
            .iter()
            .map(|label| label.label().to_string())
            .collect()
    }

    /// `true` while Apply can be pressed.
    #[cfg(test)]
    pub(super) fn can_apply(&self) -> bool {
        self.apply.is_sensitive()
    }

    /// The panel's size once laid out, to keep it inside the view.
    pub(super) fn size(&self) -> (f64, f64) {
        let (_, width, _, _) = self.root.measure(gtk::Orientation::Horizontal, -1);
        let (_, height, _, _) = self.root.measure(gtk::Orientation::Vertical, width);
        (f64::from(width), f64::from(height))
    }
}

fn report(callback: &Rc<RefCell<Option<ActionCallback>>>, action: PanelAction) {
    let callback = callback.borrow().clone();
    if let Some(callback) = callback {
        callback(action);
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// The grips' own few colours (libadwaita's named colours), installed
/// once per display.
const STYLE: &str = r#"
.stave-grip-panel { padding: 12px 14px; border-radius: 12px; background: @window_bg_color;
  color: @window_fg_color; box-shadow: 0 2px 8px alpha(black, 0.25); }
.stave-grip-title { font-weight: 700; font-family: monospace; font-size: 13px; }
.stave-grip-value { font-size: 20px; font-weight: 700; }
.stave-grip-change { font-size: 13px; font-family: monospace; color: @accent_color; }
.stave-grip-ask { font-size: 12px; font-weight: 700; color: @warning_color; }
.stave-grip-choice { font-family: monospace; font-size: 12px; }
.stave-grip-stop { padding: 6px 8px; border-radius: 8px; background: alpha(@error_color, 0.12);
  color: @error_color; font-size: 12px; font-weight: 600; }
.stave-grip-verdict { font-weight: 700; font-size: 13px; }
.stave-grip-verdict.fits { color: @success_color; }
.stave-grip-verdict.refused { color: @error_color; }
.stave-grip-verdict.checking, .stave-grip-verdict.waiting { opacity: 0.75; }
.stave-grip-handle { min-width: 16px; min-height: 16px; border-radius: 999px; background: white;
  border: 3px solid @accent_color; box-shadow: 0 1px 3px alpha(black, 0.35); }
.stave-grip-handle.follows { border-radius: 3px; border-style: double; border-width: 4px; }
.stave-grip-handle:focus-visible { outline: 2px solid alpha(@accent_color, 0.5); outline-offset: 2px; }
.stave-grip-handle.dragging { background: @accent_color; }
.stave-grip-lock { min-width: 22px; min-height: 22px; border-radius: 999px; background: @window_bg_color;
  color: @window_fg_color; box-shadow: 0 1px 3px alpha(black, 0.35); }
.stave-grip-lines { color: @accent_color; }
.stave-grip-label { padding: 1px; border-radius: 8px; background: @window_bg_color;
  color: @window_fg_color; box-shadow: 0 1px 3px alpha(black, 0.25); }
.stave-grip-label button.stave-grip-number { color: @window_fg_color; }
.stave-grip-label button.stave-grip-number { padding: 2px 8px; min-height: 22px; font-weight: 700;
  font-size: 12px; }
.stave-grip-label.follows button.stave-grip-number { font-weight: 600; font-style: italic; }
.stave-grip-label.locked { padding: 3px 8px; font-size: 12px; opacity: 0.85; }
.stave-grip-label entry { min-height: 24px; font-size: 12px; }
.stave-grip-label .stave-grip-error { color: @error_color; font-size: 12px; font-weight: 600;
  padding: 2px 6px 4px 6px; }
.stave-grip-label menubutton > button { min-height: 22px; min-width: 22px; padding: 0 2px; }
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
