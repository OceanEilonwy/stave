//! The Output panel: what the checks said, one line each, the newest at
//! the foot. The status bar's "Output" button opens it on the bottom
//! edge. It only shows the lines the session gives.
//!
//! It is a relm4 component, [`OutputPanelComponent`]: its model (the line
//! last added) is the only state and only its update changes it.
//! [`OutputPanel`] keeps the older API in front of it until the window
//! moves to relm4 too.

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use super::relm4_shim::Bridge;

/// The Output panel's state: the line last added, so a line said again
/// isn't repeated.
#[derive(Debug, Default)]
pub struct OutputPanelComponent {
    last: String,
}

/// What the Output panel is told.
#[derive(Debug)]
pub enum OutputPanelInput {
    /// Add this line at the foot, unless it is empty or the line last
    /// added.
    AddLine(String),
}

#[relm4::component(pub)]
impl Component for OutputPanelComponent {
    type Init = ();
    type Input = OutputPanelInput;
    type Output = ();
    type CommandOutput = ();

    view! {
        gtk::ScrolledWindow {
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_vexpand: true,

            #[name = "text"]
            gtk::TextView {
                set_editable: false,
                set_cursor_visible: false,
                set_monospace: true,
                set_wrap_mode: gtk::WrapMode::WordChar,
                set_left_margin: 12,
                set_right_margin: 12,
                set_top_margin: 8,
                set_bottom_margin: 8,
                add_css_class: "stave-output",
                update_property: &[gtk::accessible::Property::Label("Output")],
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = OutputPanelComponent::default();
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: OutputPanelInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            OutputPanelInput::AddLine(line) => {
                if !line.is_empty() && line != self.last {
                    append(&widgets.text, &line);
                    self.last = line;
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Writes `line` at the foot of `text` and scrolls to it.
fn append(text: &gtk::TextView, line: &str) {
    let buffer = text.buffer();
    let mut end = buffer.end_iter();
    if buffer.char_count() > 0 {
        buffer.insert(&mut end, "\n");
    }
    buffer.insert(&mut end, line);
    let mark = buffer.create_mark(None, &buffer.end_iter(), false);
    text.scroll_mark_onscreen(&mark);
    buffer.delete_mark(&mark);
}

/// The Output panel, behind the API the window still uses: it owns the
/// component (dropping it shuts the component down) and passes its
/// setter on as an [`OutputPanelInput`] message.
///
/// Lines are queued: they show when the main loop next runs, after
/// whatever added them has returned.
pub struct OutputPanel {
    /// The panel's content.
    pub root: gtk::ScrolledWindow,
    bridge: Bridge<OutputPanelComponent>,
}

impl OutputPanel {
    /// An empty panel.
    pub fn new() -> OutputPanel {
        let bridge = Bridge::<OutputPanelComponent>::launch(());
        OutputPanel {
            root: bridge.widget().clone(),
            bridge,
        }
    }

    /// Adds a line at the foot, unless it is the line last added.
    pub fn add_line(&self, line: &str) {
        self.bridge.send(OutputPanelInput::AddLine(line.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// What the panel shows.
    fn shown(panel: &OutputPanel) -> String {
        let Some(text) = panel
            .root
            .child()
            .and_then(|child| child.downcast::<gtk::TextView>().ok())
        else {
            panic!("the panel holds a text view");
        };
        let buffer = text.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string()
    }

    #[test]
    fn a_dropped_output_panel_is_freed() {
        on_gtk_thread(output_panel_is_freed);
    }

    /// Dropping the panel shuts its component down when the main loop
    /// next runs: the widget and the model's runtime are freed.
    fn output_panel_is_freed() {
        let panel = OutputPanel::new();
        let root = panel.root.downgrade();
        let input = panel.bridge.sender().clone();
        settle();
        assert!(root.upgrade().is_some(), "the widget lives while held");
        drop(panel);
        settle();
        assert!(root.upgrade().is_none(), "the widget is freed");
        assert!(
            input
                .send(OutputPanelInput::AddLine("late".to_owned()))
                .is_err(),
            "the component's runtime, and with it the model, is gone"
        );
    }

    #[test]
    fn lines_are_queued_and_a_line_said_again_is_not_repeated() {
        on_gtk_thread(lines_are_queued);
    }

    /// Adding a line only queues it: nothing shows until the main loop
    /// runs, and then each line shows once, in order, a repeat dropped.
    fn lines_are_queued() {
        let panel = OutputPanel::new();
        panel.add_line("Checked: 0 problems");
        panel.add_line("Checked: 0 problems");
        panel.add_line("");
        panel.add_line("Checked: 1 problem");
        assert_eq!(shown(&panel), "", "nothing shows until the loop runs");
        settle();
        assert_eq!(shown(&panel), "Checked: 0 problems\nChecked: 1 problem");
        assert_eq!(panel.bridge.model().last, "Checked: 1 problem");
    }
}
