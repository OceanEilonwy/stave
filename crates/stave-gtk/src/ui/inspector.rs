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
use stave_app::problems::Details;
use stave_app::session::SourceLocation;
use stave_lang::Severity;

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

    /// Shows a problem's details (Problems.dc.html): the eyebrow
    /// ("ERROR · MOTION"), a plain title, what happened, the text exactly
    /// as `stave check` prints it, and the ways to fix it, each with a
    /// "Show in text" link.
    pub fn show_problem(&self, details: &Details) {
        clear(&self.problem);
        let head = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let eyebrow = label(
            &details.eyebrow,
            &["stave-eyebrow", "stave-problem-eyebrow"],
        );
        eyebrow.add_css_class(match details.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        });
        head.append(&eyebrow);
        head.append(&label(
            &details.title,
            &["stave-inspector-title", "stave-problem-title"],
        ));
        if !details.explanation.is_empty() {
            head.append(&label(&details.explanation, &["stave-description"]));
        }
        self.problem.append(&head);

        let code = gtk::Label::builder()
            .use_markup(true)
            .label(diagnostic_markup(&details.text))
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .selectable(true)
            .css_classes(["stave-code", "stave-diagnostic"])
            .build();
        code.update_property(&[gtk::accessible::Property::Label(&details.text)]);
        self.problem.append(&code);

        if !details.fixes.is_empty() {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
            section.append(&label("WAYS TO FIX IT", &["stave-section"]));
            let card = card();
            for fix in &details.fixes {
                let line = gtk::Box::builder()
                    .spacing(10)
                    .css_classes(["stave-card-row"])
                    .build();
                line.append(
                    &gtk::Label::builder()
                        .label(&fix.text)
                        .xalign(0.0)
                        .hexpand(true)
                        .wrap(true)
                        .wrap_mode(gtk::pango::WrapMode::WordChar)
                        .build(),
                );
                if let Some(location) = fix.location.clone() {
                    line.append(&self.source_link(location));
                }
                card.append(&line);
            }
            section.append(&card);
            section.append(&label(
                "Edit the text and the check runs again when you save. Fixes are never applied without you.",
                &["stave-footnote"],
            ));
            self.problem.append(&section);
        } else if let Some(location) = details.location.clone() {
            let link = self.source_link(location);
            link.set_halign(gtk::Align::Start);
            self.problem.append(&link);
        }
        self.root.set_visible_child_name("problem");
    }

    /// A "Show in text" link to a place in the text.
    fn source_link(&self, location: SourceLocation) -> gtk::Button {
        let link = gtk::Button::builder()
            .label("Show in text")
            .css_classes(["flat", "stave-link"])
            .valign(gtk::Align::Center)
            .tooltip_text("Show where to change it, in the Files view")
            .build();
        let callbacks = self.callbacks.clone();
        link.connect_clicked(move |_| {
            let callback = callbacks.borrow().on_source.clone();
            if let Some(callback) = callback {
                callback(location.clone());
            }
        });
        link
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
fn line_markup(line: &[stave_app::inspect::Token]) -> String {
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

/// A problem as `stave check` prints it, as Pango markup: the severity and
/// code in red (amber for a warning), the arrows and gutter dim, the source
/// line highlighted, the carets red, and `help:` green.
fn diagnostic_markup(text: &str) -> String {
    let escape = |text: &str| gtk::glib::markup_escape_text(text).to_string();
    let span =
        |colour: &str, text: &str| format!("<span foreground=\"{colour}\">{}</span>", escape(text));
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if index == 0 {
            let colour = if line.starts_with("warning") {
                CODE.warning
            } else {
                CODE.error
            };
            let lines_head = match line.split_once(':') {
                Some((head, rest)) => format!(
                    "<span foreground=\"{colour}\" weight=\"500\">{}</span>:{}",
                    escape(head),
                    escape(rest)
                ),
                None => escape(line),
            };
            lines.push(lines_head);
            continue;
        }
        let trimmed = line.trim_start();
        let pad = line.get(..line.len() - trimmed.len()).unwrap_or("");
        if trimmed.starts_with("-->") || trimmed.starts_with(":::") {
            lines.push(span(CODE.comment, line));
        } else if let Some(rest) = trimmed.strip_prefix("= help:") {
            lines.push(format!(
                "{}{}",
                span(CODE.text, &format!("{pad}= help:")),
                escape(rest)
            ));
        } else if let Some(rest) = trimmed.strip_prefix("= note:") {
            lines.push(format!(
                "{}{}",
                span(CODE.comment, &format!("{pad}= note:")),
                escape(rest)
            ));
        } else if let Some((gutter, source)) = line
            .split_once(" | ")
            .or_else(|| line.strip_suffix(" |").map(|gutter| (gutter, "")))
        {
            let numbered = {
                let number = gutter.trim();
                !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
            };
            let gutter = span(CODE.comment, &format!("{gutter} |"));
            if numbered {
                let tokens = stave_app::inspect::highlight(source);
                lines.push(format!("{gutter} {}", line_markup(&tokens)));
            } else if source.trim_start().starts_with(['^', '-']) {
                let marks_end = source
                    .char_indices()
                    .find(|(_, c)| !matches!(c, ' ' | '^' | '-'))
                    .map_or(source.len(), |(i, _)| i);
                let (marks, message) = source.split_at(marks_end);
                lines.push(format!(
                    "{gutter} {}{}",
                    span(CODE.error, marks),
                    escape(message)
                ));
            } else {
                lines.push(format!("{gutter} {}", escape(source)));
            }
        } else {
            lines.push(escape(line));
        }
    }
    lines.join("\n")
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
