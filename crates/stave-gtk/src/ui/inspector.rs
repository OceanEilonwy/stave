//! The Inspector (the Details panel), drawn after Unity's inspector from
//! `stave_app::inspect::inspect`: a header row (the selection's swatch,
//! its name in a monospace field, the `file.stave:line` it is written at
//! as a link that opens the editor there, and a yellow ● when that file
//! has unsaved changes), then collapsible sections: what it is (its
//! stock, size, grain and motion), "Joined to" (its joints, the hardware
//! on it and what it turns on), "Cuts", "Checks" (only the checks that
//! fail, or "0 problems") and "Defined in" (the declaration, highlighted).
//! Rows are label | value; every value that names another part, piece of
//! hardware, assembly or space is a link that selects it on a click and
//! lights it in the 3D view while the pointer is on it. A joint's or a
//! cut's name opens focus mode on it. In focus mode it shows focus mode's
//! own panel. When there is nothing to select (a library folder, a design
//! that doesn't solve) it says why, from `stave_app::empty`.
//!
//! ```text
//! [▪] [stile[hinge]               ]  doors.stave:9 ●
//! ▾ Part                                   in door_l
//!   Stock         oak_19 · White oak, 19 mm
//!   Moves         with door_l, turns 110°
//! ▾ Joined to                                     4
//!   cope_and_stick (rail[bottom]) (rail[top])
//!   euro_hinge ×2  (hinges[lower]) (hinges[upper])  back face
//! ▸ Checks                                0 problems
//! ```

use stave_app::i18n::{fill_in, ntr, tr};
use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::empty::Note;
use stave_app::focus::Target;
use stave_app::inspect::{
    Connection, ConnectionSection, Field, Inspection, LinkTarget, Snippet, Swatch, SwatchShape,
    TokenClass,
};
use stave_app::session::{Selection, SourceLocation};
use stave_lang::Severity;

use super::focus_panel::FocusPanel;
use super::icons;
use super::sections::{self, LinkHandlers};
use super::style::CODE;

type TargetCallback = Rc<dyn Fn(Target)>;
type SourceCallback = Rc<dyn Fn(SourceLocation)>;
type UnsavedCallback = Rc<dyn Fn(&str) -> bool>;

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
    /// Where the links it draws report clicks and hovers.
    pub links: LinkHandlers,
    callbacks: Rc<RefCell<Callbacks>>,
}

#[derive(Default)]
struct Callbacks {
    on_focus: Option<TargetCallback>,
    on_source: Option<SourceCallback>,
    unsaved: Option<UnsavedCallback>,
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

/// A swatch's mark, `size` pixels.
pub(super) fn swatch_mark(swatch: &Swatch, size: i32) -> gtk::DrawingArea {
    let colour = gtk::gdk::RGBA::new(
        f32::from(swatch.colour.red) / 255.0,
        f32::from(swatch.colour.green) / 255.0,
        f32::from(swatch.colour.blue) / 255.0,
        1.0,
    );
    icons::swatch(colour, swatch.shape == SwatchShape::Round, size)
}

/// "0 problems", "1 problem", "2 problems".
fn problems_note(count: usize) -> String {
    fill_in(
        &ntr("{count} problem", "{count} problems", count),
        &[("count", &count)],
    )
}

impl Inspector {
    /// An empty inspector.
    pub fn new() -> Inspector {
        sections::install_style();
        let empty = adw::StatusPage::builder()
            .title(tr("Nothing selected"))
            .description(tr("Click a part in the list or in the 3D view to see its stock, size, joints and where it is written."))
            .css_classes(["compact"])
            .build();
        let details = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .css_classes(["stave-inspector-compact"])
            .build();
        let focus = FocusPanel::new();
        let focus_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-inspector"])
            .build();
        focus_box.append(&focus.root);
        let root = gtk::Stack::builder()
            .css_classes(["stave-inspector-panel"])
            .width_request(240)
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
            links: LinkHandlers::default(),
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

    /// Asks `callback` whether a file (by its path) has unsaved changes,
    /// for the ● after the file:line link.
    pub fn connect_unsaved(&self, callback: impl Fn(&str) -> bool + 'static) {
        self.callbacks.borrow_mut().unsaved = Some(Rc::new(callback));
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
        self.details.append(&self.header(inspection));

        // What it is: its kind (and where it sits) and its rows.
        let place = inspection
            .within
            .as_ref()
            .map_or_else(String::new, |assembly| {
                fill_in(&tr("in {assembly}"), &[("assembly", assembly)])
            });
        let (section_box, body) = sections::section(&inspection.kind, &place, true);
        if let Some(description) = &inspection.description {
            body.append(&sections::words(description, &["stave-row-note"]));
        }
        for field in &inspection.rows {
            body.append(&self.field_row(field));
        }
        self.details.append(&section_box);

        let joined: Vec<&Connection> = inspection
            .connections
            .iter()
            .filter(|c| c.section == ConnectionSection::Joined)
            .collect();
        if !joined.is_empty() {
            let (section_box, body) =
                sections::section(&tr("Joined to"), &joined.len().to_string(), true);
            for connection in joined {
                body.append(&self.connection_row(connection));
            }
            self.details.append(&section_box);
        }
        let cuts: Vec<&Connection> = inspection
            .connections
            .iter()
            .filter(|c| c.section == ConnectionSection::Cuts)
            .collect();
        if !cuts.is_empty() {
            let (section_box, body) = sections::section(&tr("Cuts"), &cuts.len().to_string(), true);
            for connection in cuts {
                body.append(&self.connection_row(connection));
            }
            self.details.append(&section_box);
        }
        let how: Vec<&Connection> = inspection
            .connections
            .iter()
            .filter(|c| c.section == ConnectionSection::HowItWorks)
            .collect();
        if !how.is_empty() {
            let (section_box, body) =
                sections::section(&tr(inspection.connections_title), "", true);
            for connection in how {
                body.append(&self.connection_row(connection));
            }
            self.details.append(&section_box);
        }
        self.details.append(&self.checks_section(inspection));
        if let Some(source) = &inspection.source {
            self.details.append(&self.source_section(source));
        }
        self.root.set_visible_child_name("details");
    }

    /// The header row: swatch, name, and the file:line link (with ● when
    /// that file has unsaved changes).
    fn header(&self, inspection: &Inspection) -> gtk::Box {
        let swatch = inspection
            .swatch
            .as_ref()
            .map(|swatch| swatch_mark(swatch, 12).upcast::<gtk::Widget>());
        let name = sections::name_field(&inspection.title);
        let trailing = inspection.source.as_ref().map(|source| {
            let holder = gtk::Box::builder().spacing(2).build();
            holder.append(&self.source_link(source));
            let unsaved = self.callbacks.borrow().unsaved.clone();
            if unsaved.is_some_and(|unsaved| unsaved(&source.location.path)) {
                holder.append(
                    &gtk::Label::builder()
                        .label("●")
                        .tooltip_text(tr("This file has changes that aren't saved"))
                        .css_classes(["stave-unsaved-dot"])
                        .build(),
                );
            }
            holder.upcast::<gtk::Widget>()
        });
        sections::header(swatch.as_ref(), &name, trailing.as_ref())
    }

    /// The `file.stave:line` link that opens the editor there.
    fn source_link(&self, source: &Snippet) -> gtk::Button {
        let link = super::style::as_link(
            gtk::Button::builder()
                .label(&source.label)
                .css_classes(["flat", "stave-link", "stave-source-link"])
                .tooltip_text(tr("Show it in the editor"))
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
        link
    }

    /// A label | value row; a value that names other things shows them
    /// as links.
    fn field_row(&self, field: &Field) -> gtk::Box {
        if field.links.is_empty() {
            return sections::row(&tr(field.label), &sections::value(&field.value));
        }
        let links = self.links.links(&field.links);
        links.set_tooltip_text(Some(&field.value));
        sections::row(&tr(field.label), &links)
    }

    /// A joint, hardware or cut row: its name on the left (opening focus
    /// mode when it can be shown), then what it joins as links, or its
    /// size.
    fn connection_row(&self, connection: &Connection) -> gtk::Box {
        let line = gtk::Box::builder()
            .spacing(6)
            .css_classes(["stave-field-row"])
            .build();
        let lead = gtk::Label::builder()
            .label(&connection.lead)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .tooltip_text(&connection.lead)
            .css_classes(["stave-field-label", "stave-row-lead"])
            .build();
        if connection.section == ConnectionSection::Joined {
            lead.add_css_class("joint");
        }
        let lead: gtk::Widget = match connection.target {
            Some(target) => {
                let button = gtk::Button::builder()
                    .child(&lead)
                    .css_classes(["flat", "stave-row-lead-button"])
                    .tooltip_text(match connection.section {
                        ConnectionSection::Joined | ConnectionSection::HowItWorks => {
                            tr("See how it goes together")
                        }
                        ConnectionSection::Cuts => tr("See the hardware that makes it"),
                    })
                    .build();
                let callbacks = self.callbacks.clone();
                button.connect_clicked(move |_| {
                    let callback = callbacks.borrow().on_focus.clone();
                    if let Some(callback) = callback {
                        callback(target);
                    }
                });
                button.upcast()
            }
            None => lead.upcast(),
        };
        line.append(&lead);
        if !connection.partners.is_empty() {
            let links = self.links.links(&connection.partners);
            links.set_tooltip_text(Some(&format!("{} {}", connection.lead, connection.rest)));
            line.append(&links);
            if connection.section == ConnectionSection::Joined && !connection.note.is_empty() {
                line.append(
                    &gtk::Label::builder()
                        .label(&connection.note)
                        .css_classes(["stave-row-note"])
                        .build(),
                );
            }
        } else if !connection.note.is_empty() {
            let value = sections::value(&connection.note);
            value.set_hexpand(true);
            line.append(&value);
        }
        line
    }

    /// "Checks": each check that fails about it (a click selects the
    /// problem), or a count of none.
    fn checks_section(&self, inspection: &Inspection) -> gtk::Box {
        let count = inspection.checks.len();
        let (section_box, body) =
            sections::section(&tr("Checks"), &problems_note(count), count > 0);
        if count == 0 {
            section_box.add_css_class("empty");
            return section_box;
        }
        for check in &inspection.checks {
            let (icon, class) = match check.severity {
                Severity::Error => ("dialog-error-symbolic", "error"),
                Severity::Warning | Severity::Note => ("dialog-warning-symbolic", "warning"),
            };
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let image = gtk::Image::from_icon_name(icon);
            image.set_valign(gtk::Align::Start);
            content.append(&image);
            content.append(&sections::words(&check.message, &[]));
            let button = gtk::Button::builder()
                .child(&content)
                .css_classes(["flat", "stave-check-row", class])
                .tooltip_text(tr("Show this problem"))
                .build();
            let links = self.links.clone();
            let problem = check.problem;
            button.connect_clicked(move |_| {
                links.click_target(LinkTarget::Select(Selection::Problem(problem)));
            });
            body.append(&button);
        }
        section_box
    }

    fn source_section(&self, source: &Snippet) -> gtk::Box {
        let (section_box, section) = sections::section(&tr("Defined in"), &source.label, true);
        // One label per line, each ending in "…" where it doesn't fit, so
        // no letter is ever cut in half.
        let code = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
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
        section.append(&sections::words(
            &tr("The text is the source of truth. Positions are worked out from these relationships; none are stored."),
            &["stave-footnote"],
        ));
        section_box
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

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_are_counted_in_words() {
        assert_eq!(problems_note(0), "0 problems");
        assert_eq!(problems_note(1), "1 problem");
    }
}
