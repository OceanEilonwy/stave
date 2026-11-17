//! Focus mode's side panel (JointFocus.dc.html, Hinge.dc.html): for a
//! joint, its name and description, Goes in / Receives it / the cut / Fit,
//! the X-ray, Tint cut faces and Fade other parts toggles, and its insert
//! steps as written with a link to the kind; for hardware, "How this
//! hinge works", its numbered members and Opens to / Closest to the side /
//! Swing check. Everything it shows comes from `stave_app::focus::Focus`;
//! this module only lays it out. The step player, the swing slider and the
//! small pictures are in the 3D view's area (`focus_stage`).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::focus::Focus;
use stave_app::session::SourceLocation;

use super::icons::{self, PathIcon};
use super::style::CODE;

type SourceCallback = Rc<dyn Fn(SourceLocation)>;

/// The focus panel's widgets.
pub struct FocusPanel {
    /// The panel, ready to put in the side panel.
    pub root: gtk::Box,
    eyebrow: gtk::Label,
    heading: gtk::Label,
    description: gtk::Label,
    more: gtk::Expander,
    more_text: gtk::Label,
    /// Leaves focus mode.
    pub done: gtk::Button,
    members: gtk::Box,
    rows: gtk::Box,
    show_section: gtk::Box,
    /// Draws the receiving part see-through.
    pub xray: gtk::CheckButton,
    xray_label: gtk::Label,
    /// Tints the faces cuts made.
    pub tint: gtk::CheckButton,
    /// Fades the other parts.
    pub fade: gtk::CheckButton,
    insert_section: gtk::Box,
    insert_link: gtk::Button,
    insert_code: gtk::Label,
    insert_location: Rc<RefCell<Option<SourceLocation>>>,
    on_source: Rc<RefCell<Option<SourceCallback>>>,
}

fn label(classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(classes.to_vec())
        .build()
}

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

/// A toggle row: bold title, dim words, a check box at the end.
fn toggle_row(title: &str, words: &str) -> (gtk::Box, gtk::CheckButton, gtk::Label) {
    let text = gtk::Label::builder()
        .use_markup(true)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    set_toggle_text(&text, title, words);
    let check = gtk::CheckButton::builder()
        .active(true)
        .valign(gtk::Align::Center)
        .build();
    check.update_property(&[gtk::accessible::Property::Label(title)]);
    let row = gtk::Box::builder()
        .spacing(10)
        .css_classes(["stave-card-row", "stave-toggle-row"])
        .build();
    row.append(&text);
    row.append(&check);
    // Clicking anywhere on the row toggles it.
    let click = gtk::GestureClick::new();
    let target = check.clone();
    click.connect_released(move |_, _, _, _| target.set_active(!target.is_active()));
    text.add_controller(click);
    (row, check, text)
}

fn set_toggle_text(text: &gtk::Label, title: &str, words: &str) {
    let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
    text.set_markup(&if words.is_empty() {
        format!("<b>{}</b>", escape(title))
    } else {
        format!(
            "<b>{}</b> <span alpha=\"70%\">{}</span>",
            escape(title),
            escape(words)
        )
    });
}

impl FocusPanel {
    /// An empty, hidden panel.
    pub fn new() -> FocusPanel {
        let eyebrow = label(&["stave-eyebrow", "stave-eyebrow-bold"]);
        eyebrow.set_hexpand(true);
        let done = gtk::Button::builder()
            .child(&PathIcon::new(icons::CLOSE, 12))
            .tooltip_text("Leave focus mode (Esc)")
            .css_classes(["flat", "circular", "stave-close"])
            .valign(gtk::Align::Center)
            .build();
        done.update_property(&[gtk::accessible::Property::Label("Leave focus mode")]);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.append(&eyebrow);
        top.append(&done);
        let heading = label(&["stave-inspector-title"]);
        let description = label(&["stave-description"]);
        let head = gtk::Box::new(gtk::Orientation::Vertical, 4);
        head.append(&top);
        head.append(&heading);
        head.append(&description);
        let more_text = label(&["stave-description"]);
        let more = gtk::Expander::builder()
            .label("More")
            .child(&more_text)
            .css_classes(["stave-more"])
            .visible(false)
            .build();
        head.append(&more);

        let members = card();
        let rows = card();

        let (xray_row, xray, xray_label) = toggle_row("X-ray", "see into the part");
        let (tint_row, tint, _) = toggle_row("Tint cut faces", "holes, mortises, dados");
        let (fade_row, fade, _) = toggle_row("Fade other parts", "");
        let toggles = card();
        toggles.append(&xray_row);
        toggles.append(&tint_row);
        toggles.append(&fade_row);
        let show_title = label(&["stave-section"]);
        show_title.set_label("SHOW");
        let show_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        show_section.append(&show_title);
        show_section.append(&toggles);

        let insert_link = super::style::as_link(
            gtk::Button::builder()
                .css_classes(["flat", "stave-link"])
                .tooltip_text("Show the joint's kind in the text")
                .valign(gtk::Align::Center)
                .build(),
        );
        let insert_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let insert_title = label(&["stave-section"]);
        insert_title.set_label("HOW IT GOES TOGETHER");
        insert_title.set_hexpand(true);
        insert_title.set_wrap(false);
        insert_title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        insert_head.append(&insert_title);
        insert_head.append(&insert_link);
        let insert_code = gtk::Label::builder()
            .use_markup(true)
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .css_classes(["stave-code"])
            .build();
        let insert_note = label(&["stave-footnote"]);
        insert_note.set_label("The animation plays the joint’s own insert steps, the same ones the assembly check runs backwards. What you see is what was checked.");
        let insert_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        insert_section.append(&insert_head);
        insert_section.append(&insert_code);
        insert_section.append(&insert_note);

        let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
        root.set_visible(false);
        root.append(&head);
        root.append(&members);
        root.append(&rows);
        root.append(&show_section);
        root.append(&insert_section);

        let panel = FocusPanel {
            root,
            eyebrow,
            heading,
            description,
            more,
            more_text,
            done,
            members,
            rows,
            show_section,
            xray,
            xray_label,
            tint,
            fade,
            insert_section,
            insert_link,
            insert_code,
            insert_location: Rc::new(RefCell::new(None)),
            on_source: Rc::new(RefCell::new(None)),
        };
        let location = panel.insert_location.clone();
        let callback = panel.on_source.clone();
        panel.insert_link.connect_clicked(move |_| {
            let location = location.borrow().clone();
            let callback = callback.borrow().clone();
            if let (Some(location), Some(callback)) = (location, callback) {
                callback(location);
            }
        });
        panel
    }

    /// Calls `callback` when the link to the joint's kind is clicked.
    pub fn connect_source(&self, callback: impl Fn(SourceLocation) + 'static) {
        self.on_source.replace(Some(Rc::new(callback)));
    }

    /// Shows a focus, or hides the panel (`None`).
    pub fn show(&self, focus: Option<&Focus>) {
        clear(&self.members);
        clear(&self.rows);
        let Some(focus) = focus else {
            self.root.set_visible(false);
            return;
        };
        self.root.set_visible(true);
        let joint = focus.assembles();
        self.eyebrow
            .set_label(if joint { "JOINT" } else { "HARDWARE" });
        self.heading.set_label(&focus.heading);
        // The first paragraph (two sentences at most), the rest under More.
        let full = focus.description.as_deref().unwrap_or(&focus.title);
        let summary = focus.summary.as_deref().unwrap_or(full);
        self.description.set_label(summary);
        self.description.set_tooltip_text(Some(&focus.title));
        let rest = full.strip_prefix(summary).unwrap_or(full).trim();
        self.more_text.set_label(rest);
        self.more.set_expanded(false);
        self.more.set_visible(!rest.is_empty() && summary != full);

        // A hardware kind's members, numbered as on the drawing.
        for member in &focus.members {
            let line = gtk::Box::builder()
                .spacing(10)
                .css_classes(["stave-card-row", "compact"])
                .build();
            let number = gtk::Label::builder()
                .label(member.number.to_string())
                .css_classes(["stave-number"])
                .valign(gtk::Align::Start)
                .build();
            line.append(&number);
            let name = member_title(&member.name);
            let text = gtk::Label::builder()
                .use_markup(true)
                .label(if member.caption.is_empty() {
                    format!("<b>{}</b>", gtk::glib::markup_escape_text(&name))
                } else {
                    format!(
                        "<b>{}</b> · {}",
                        gtk::glib::markup_escape_text(&name),
                        gtk::glib::markup_escape_text(&member.caption)
                    )
                })
                .xalign(0.0)
                .hexpand(true)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .lines(2)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .tooltip_text(&member.caption)
                .build();
            line.append(&text);
            self.members.append(&line);
        }
        self.members.set_visible(!focus.members.is_empty());

        // Goes in, Receives it, the cut, Fit; or Opens to, Closest, Swing check.
        for entry in &focus.rows {
            let line = gtk::Box::builder()
                .spacing(12)
                .css_classes(["stave-card-row"])
                .build();
            line.append(
                &gtk::Label::builder()
                    .label(entry.label)
                    .xalign(0.0)
                    .valign(gtk::Align::Start)
                    .css_classes(["stave-key"])
                    .build(),
            );
            let value = gtk::Label::builder()
                .label(&entry.value)
                .xalign(1.0)
                .hexpand(true)
                .wrap(true)
                .justify(gtk::Justification::Right)
                .css_classes(["stave-value"])
                .build();
            if matches!(entry.label, "Fit" | "Swing check" | "Slide check") {
                value.add_css_class(
                    if entry.value.starts_with("touches") || entry.value.starts_with("clear") {
                        "good"
                    } else {
                        "bad"
                    },
                );
            }
            line.append(&value);
            self.rows.append(&line);
        }
        self.rows.set_visible(!focus.rows.is_empty());

        self.show_section.set_visible(joint);
        let receives = focus.receives.as_deref().unwrap_or("the part");
        set_toggle_text(&self.xray_label, "X-ray", &format!("see into {receives}"));

        match &focus.insert {
            Some(insert) => {
                self.insert_section.set_visible(true);
                self.insert_link.set_label(&insert.link);
                self.insert_link.set_visible(insert.location.is_some());
                self.insert_location.replace(insert.location.clone());
                self.insert_code.set_markup(&insert_markup(&insert.steps));
                self.insert_code
                    .update_property(&[gtk::accessible::Property::Label(&insert.text)]);
            }
            None => self.insert_section.set_visible(false),
        }
    }
}

/// A member's name for the list: `cabinet_member` → "Cabinet member".
fn member_title(name: &str) -> String {
    let spaced = name.replace('_', " ");
    let mut chars = spaced.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// `insert: [ push(piece, along: length), ]` laid out one step a line, as
/// `stave fmt` writes it, coloured.
fn insert_markup(steps: &[String]) -> String {
    let escape = |t: &str| gtk::glib::markup_escape_text(t).to_string();
    let mut out = format!("<span foreground=\"{}\">insert</span>: [", CODE.keyword);
    for step in steps {
        let (call, rest) = step.split_once('(').unwrap_or((step.as_str(), ""));
        out.push_str(&format!(
            "\n  <span foreground=\"{}\">{}</span>{}{},",
            CODE.name,
            escape(call),
            if rest.is_empty() && !step.contains('(') {
                ""
            } else {
                "("
            },
            escape(rest)
        ));
    }
    out.push_str("\n]");
    out
}
