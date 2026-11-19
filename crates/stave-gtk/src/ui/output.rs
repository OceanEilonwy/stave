//! The Output panel: what the checks said, one line each, the newest at
//! the foot. The status bar's "Output" button opens it on the bottom
//! edge. It only shows the lines the session gives.

use adw::prelude::*;
use stave_app::i18n::tr;

/// The Output panel.
pub struct OutputPanel {
    /// The panel's content.
    pub root: gtk::ScrolledWindow,
    text: gtk::TextView,
    /// The last line written, so a line said again isn't repeated.
    last: std::cell::RefCell<String>,
}

impl OutputPanel {
    /// An empty panel.
    pub fn new() -> OutputPanel {
        let text = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(12)
            .right_margin(12)
            .top_margin(8)
            .bottom_margin(8)
            .css_classes(["stave-output"])
            .build();
        text.update_property(&[gtk::accessible::Property::Label(&tr("Output"))]);
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&text)
            .build();
        OutputPanel {
            root,
            text,
            last: std::cell::RefCell::new(String::new()),
        }
    }

    /// Adds a line at the foot, unless it is the line last added.
    pub fn add_line(&self, line: &str) {
        if line.is_empty() || *self.last.borrow() == line {
            return;
        }
        self.last.replace(line.to_owned());
        let buffer = self.text.buffer();
        let mut end = buffer.end_iter();
        if buffer.char_count() > 0 {
            buffer.insert(&mut end, "\n");
        }
        buffer.insert(&mut end, line);
        let mark = buffer.create_mark(None, &buffer.end_iter(), false);
        self.text.scroll_mark_onscreen(&mark);
        buffer.delete_mark(&mark);
    }
}
