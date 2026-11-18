//! The inspector: what the selection is (from
//! `stave_app::inspect::inspect`), its stock, size and motion, its joints
//! and cuts (each opens focus mode), and where it is written in the text
//! (the link opens the editor there). In focus mode it shows focus
//! mode's own panel. A selected problem is read in the Problems panel,
//! not here. When there is nothing to select (a library folder, a design
//! that doesn't solve) it says why, from `stave_app::empty`.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::empty::Note;
use stave_app::focus::Target;
use stave_app::inspect::{Inspection, Snippet, TokenClass};
use stave_app::session::SourceLocation;

use super::focus_panel::FocusPanel;
use super::icons;
use super::style::CODE;

type TargetCallback = Rc<dyn Fn(Target)>;
type SourceCallback = Rc<dyn Fn(SourceLocation)>;

/// The inspector panel.
pub struct Inspector {
    /// The panel, ready to put in the split view.
    pub root: gtk::Stack,
    details: gtk::Box,
    /// Focus mode's panel.
    pub focus: FocusPanel,
    /// Why there is nothing to select, shown instead of "Nothing selected".
    note: gtk::Box,
    idle: RefCell<Option<Note>>,
    callbacks: Rc<RefCell<Callbacks>>,
}

#[derive(Default)]
struct Callbacks {
    on_focus: Option<TargetCallback>,
    on_source: Option<SourceCallback>,
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(classes.to_vec())
        .build()
}

/// A page of the card: as tall as it needs, scrolling past the room the
/// view leaves it.
fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(640)
        .child(child)
        .build()
}

impl Inspector {
    /// An empty inspector.
    pub fn new() -> Inspector {
        let empty = adw::StatusPage::builder()
            .title("Nothing selected")
            .description("Click a part in the list or in the 3D view to see its stock, size, joints and where it is written.")
            .css_classes(["compact"])
            .build();
        let details = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .css_classes(["stave-inspector"])
            .build();
        let focus = FocusPanel::new();
        let focus_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-inspector"])
            .build();
        focus_box.append(&focus.root);
        let root = gtk::Stack::builder()
            .css_classes(["stave-inspector-panel"])
            .width_request(300)
            .vhomogeneous(false)
            .interpolate_size(true)
            .build();
        root.add_named(&empty, Some("empty"));
        root.add_named(&scrolled(&details), Some("details"));
        root.add_named(&scrolled(&focus_box), Some("focus"));
        let note = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .css_classes(["stave-inspector"])
            .build();
        root.add_named(&scrolled(&note), Some("note"));
        root.set_visible_child_name("empty");
        Inspector {
            note,
            idle: RefCell::new(None),
            root,
            details,
            focus,
            callbacks: Rc::new(RefCell::new(Callbacks::default())),
        }
    }

    /// Calls `callback` when a joint or hardware row asks for focus mode.
    pub fn connect_focus(&self, callback: impl Fn(Target) + 'static) {
        self.callbacks.borrow_mut().on_focus = Some(Rc::new(callback));
    }

    /// Calls `callback` when the file:line link is clicked.
    pub fn connect_source(&self, callback: impl Fn(SourceLocation) + 'static) {
        self.callbacks.borrow_mut().on_source = Some(Rc::new(callback));
    }

    /// Shows focus mode's panel (it fills itself).
    pub fn show_focus(&self) {
        self.root.set_visible_child_name("focus");
    }

    /// Sets what to say when nothing is selected because there is nothing
    /// to select (`None`: the usual "Nothing selected" hint), and shows it
    /// if nothing is shown.
    pub fn show_idle(&self, note: Option<Note>) {
        if let Some(note) = &note {
            clear(&self.note);
            let head = gtk::Box::new(gtk::Orientation::Vertical, 6);
            head.append(&label(&note.title, &["stave-inspector-title"]));
            if !note.description.is_empty() {
                head.append(&label(&note.description, &["stave-description"]));
            }
            self.note.append(&head);
            if let Some(code) = &note.code {
                let block = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .css_classes(["stave-code"])
                    .build();
                block.append(
                    &gtk::Label::builder()
                        .use_markup(true)
                        .label(line_markup(&stave_app::inspect::highlight(code)))
                        .xalign(0.0)
                        .wrap(true)
                        .wrap_mode(gtk::pango::WrapMode::Char)
                        .build(),
                );
                self.note.append(&block);
            }
            if let Some(after) = &note.after {
                self.note.append(&label(after, &["stave-description"]));
            }
        }
        let idle = note.is_some();
        self.idle.replace(note);
        let showing = self.root.visible_child_name();
        if matches!(showing.as_deref(), Some("empty" | "note")) {
            self.root
                .set_visible_child_name(if idle { "note" } else { "empty" });
        }
    }

    /// Shows an inspection, or the hint when nothing is selected.
    pub fn show(&self, inspection: Option<&Inspection>) {
        let Some(inspection) = inspection else {
            let idle = self.idle.borrow().is_some();
            self.root
                .set_visible_child_name(if idle { "note" } else { "empty" });
            return;
        };
        clear(&self.details);
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head.append(&label(&inspection.eyebrow, &["stave-eyebrow"]));
        head.append(&label(&inspection.title, &["stave-inspector-title"]));
        if let Some(description) = &inspection.description {
            head.append(&label(description, &["stave-description"]));
        }
        self.details.append(&head);

        if !inspection.rows.is_empty() {
            let card = card();
            for row in &inspection.rows {
                let line = gtk::Box::builder().css_classes(["stave-card-row"]).build();
                line.append(
                    &gtk::Label::builder()
                        .label(row.label)
                        .xalign(0.0)
                        .valign(gtk::Align::Start)
                        .css_classes(["stave-key"])
                        .build(),
                );
                line.append(
                    &gtk::Label::builder()
                        .label(&row.value)
                        .xalign(1.0)
                        .hexpand(true)
                        .wrap(true)
                        .justify(gtk::Justification::Right)
                        .css_classes(["stave-value"])
                        .build(),
                );
                card.append(&line);
            }
            self.details.append(&card);
        }

        if !inspection.connections.is_empty() {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
            section.append(&label(
                &inspection.connections_title.to_uppercase(),
                &["stave-section"],
            ));
            let card = card();
            for connection in &inspection.connections {
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let colour = gtk::gdk::RGBA::new(
                    f32::from(connection.swatch.colour.red) / 255.0,
                    f32::from(connection.swatch.colour.green) / 255.0,
                    f32::from(connection.swatch.colour.blue) / 255.0,
                    1.0,
                );
                content.append(&icons::swatch(colour, true, 8));
                let text = gtk::Label::builder()
                    .use_markup(true)
                    .label(format!(
                        "<b>{}</b> {}",
                        gtk::glib::markup_escape_text(&connection.lead),
                        gtk::glib::markup_escape_text(&connection.rest)
                    ))
                    .xalign(0.0)
                    .hexpand(true)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .build();
                content.append(&text);
                match connection.target {
                    Some(target) => {
                        let button = gtk::Button::builder()
                            .child(&content)
                            .css_classes(["stave-card-row", "flat"])
                            .tooltip_text("See how it goes together")
                            .build();
                        let callbacks = self.callbacks.clone();
                        button.connect_clicked(move |_| {
                            let callback = callbacks.borrow().on_focus.clone();
                            if let Some(callback) = callback {
                                callback(target);
                            }
                        });
                        card.append(&button);
                    }
                    None => {
                        content.add_css_class("stave-card-row");
                        card.append(&content);
                    }
                }
            }
            section.append(&card);
            self.details.append(&section);
        }

        if let Some(source) = &inspection.source {
            self.details.append(&self.source_section(source));
        }
        self.root.set_visible_child_name("details");
    }

    fn source_section(&self, source: &Snippet) -> gtk::Box {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        heading.append(
            &gtk::Label::builder()
                .label("DEFINED IN THE TEXT")
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["stave-section"])
                .build(),
        );
        let link = super::style::as_link(
            gtk::Button::builder()
                .label(&source.label)
                .css_classes(["flat", "stave-link"])
                .tooltip_text("Show it in the editor")
                .build(),
        );
        let location = source.location.clone();
        let callbacks = self.callbacks.clone();
        link.connect_clicked(move |_| {
            let callback = callbacks.borrow().on_source.clone();
            if let Some(callback) = callback {
                callback(location.clone());
            }
        });
        heading.append(&link);
        section.append(&heading);
        // One label per line, each ending in "…" where it doesn't fit, so
        // no letter is ever cut in half.
        let code = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["stave-code"])
            .build();
        let mut lines: Vec<String> = source.lines.iter().map(|line| line_markup(line)).collect();
        if source.cut_short {
            lines.push("…".to_owned());
        }
        for markup in lines {
            code.append(
                &gtk::Label::builder()
                    .use_markup(true)
                    .label(if markup.is_empty() {
                        " "
                    } else {
                        markup.as_str()
                    })
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
        }
        section.append(&code);
        section.append(&label(
            "The text is the source of truth. Positions are worked out from these relationships; none are stored.",
            &["stave-footnote"],
        ));
        section
    }
}

/// One line of a snippet as Pango markup, coloured by token.
pub(super) fn line_markup(line: &[stave_app::inspect::Token]) -> String {
    let mut markup = String::new();
    for token in line {
        let text = gtk::glib::markup_escape_text(&token.text);
        let colour = match token.class {
            TokenClass::Plain => None,
            TokenClass::Keyword => Some(CODE.keyword),
            TokenClass::Type | TokenClass::Function => Some(CODE.name),
            TokenClass::Number => Some(CODE.number),
            TokenClass::Text => Some(CODE.text),
            TokenClass::Comment => Some(CODE.comment),
        };
        match colour {
            Some(colour) => {
                markup.push_str(&format!("<span foreground=\"{colour}\">{text}</span>"));
            }
            None => markup.push_str(&text),
        }
    }
    markup
}

/// A rounded card holding rows.
fn card() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-card"])
        .overflow(gtk::Overflow::Hidden)
        .build()
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
