//! The inspector: what the selection is (from
//! `stave_app::inspect::inspect`), its stock, size and motion, its joints
//! and cuts (each opens focus mode), and where it is written in the text
//! (the link opens the Files view there). In focus mode it shows focus
//! mode's own panel; for a selected problem it is the slot lane 3 fills.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::focus::Target;
use stave_app::inspect::{Inspection, Snippet, TokenClass};
use stave_app::session::SourceLocation;
use stave_app::view::Problem;

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
    problem: gtk::Box,
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

fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(child)
        .vexpand(true)
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
        let problem = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(16)
            .css_classes(["stave-inspector"])
            .build();
        let root = gtk::Stack::builder()
            .css_classes(["stave-inspector-panel"])
            .width_request(300)
            .build();
        root.add_named(&empty, Some("empty"));
        root.add_named(&scrolled(&details), Some("details"));
        root.add_named(&scrolled(&focus_box), Some("focus"));
        root.add_named(&scrolled(&problem), Some("problem"));
        root.set_visible_child_name("empty");
        Inspector {
            root,
            details,
            focus,
            problem,
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

    /// The problem slot: lane 3 replaces this with the problem's details.
    /// For now it shows the message, the code, where and the help line.
    pub fn show_problem(&self, problem: &Problem) {
        clear(&self.problem);
        let eyebrow = format!(
            "{} · {}",
            problem.severity.word().to_uppercase(),
            problem.code.to_uppercase()
        );
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head.append(&label(&eyebrow, &["stave-eyebrow"]));
        head.append(&label(&problem.message, &["stave-inspector-title"]));
        self.problem.append(&head);
        if let Some(help) = &problem.help {
            self.problem
                .append(&label(&format!("help: {help}"), &["stave-description"]));
        }
        if let Some(location) = &problem.location {
            self.problem.append(&label(location, &["stave-footnote"]));
        }
        self.root.set_visible_child_name("problem");
    }

    /// Shows an inspection, or the hint when nothing is selected.
    pub fn show(&self, inspection: Option<&Inspection>) {
        let Some(inspection) = inspection else {
            self.root.set_visible_child_name("empty");
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
        let link = gtk::Button::builder()
            .label(&source.label)
            .css_classes(["flat", "stave-link"])
            .tooltip_text("Show it in the Files view")
            .build();
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
        let mut markup = String::new();
        for (index, line) in source.lines.iter().enumerate() {
            if index > 0 {
                markup.push('\n');
            }
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
        }
        if source.cut_short {
            markup.push_str("\n…");
        }
        let code = gtk::Label::builder()
            .use_markup(true)
            .label(&markup)
            .xalign(0.0)
            .build();
        let attributes = gtk::pango::AttrList::new();
        attributes.insert(gtk::pango::AttrFloat::new_line_height(1.6));
        code.set_attributes(Some(&attributes));
        let frame = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::External)
            .propagate_natural_height(true)
            .child(&code)
            .css_classes(["stave-code"])
            .build();
        section.append(&frame);
        section.append(&label(
            "The text is the source of truth. Positions are worked out from these relationships; none are stored.",
            &["stave-footnote"],
        ));
        section
    }
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
