//! The Inspector's and the Edit panel's building blocks, drawn after
//! Unity's inspector (`docs/gui.md`, "Details"): a header row (swatch,
//! the name in a monospace field, a `file.stave:line` link), collapsible
//! sections (a chevron and a bold title on a faint band, a grey note on
//! the right), label | value rows about 22 px tall with a 104 px grey
//! label column, and links for every value that names another thing of
//! the design (monospace, light blue on a faint blue tint): a click
//! selects it, the pointer on it lights it in the 3D view.
//!
//! ```text
//! [▪] [stile[hinge]          ]   doors.stave:9 ●
//! ▾ Part
//!   Stock        oak_19 · White oak, 19 mm
//! ▾ Joined to                             4
//!   cope_and_stick  (rail[bottom]) (rail[top])
//! ```
//!
//! Only widgets and their look live here; what they say comes from
//! `stave_app`.
//!
//! Nothing here is a relm4 component: these are widget builders used
//! inside the Inspector's and the Edit panel's views
//! (docs/relm4-migration.md, the leaf table). A section keeps whether it
//! is open in its own revealer, and [`LinkHandlers`] is only where every
//! link a panel draws sends its clicks and hovers: its owner's own input,
//! as the owner's messages.

use stave_app::i18n::{fill_in, tr, tr_noop};

use adw::prelude::*;
use gtk::gdk;
use stave_app::inspect::{LinkTarget, NameLink};

/// Where the links a component draws report: its own input, with a click
/// and a hover each made into one of its messages. The links only send;
/// the component decides what to tell its owner.
pub(super) struct LinkHandlers<Message> {
    input: relm4::Sender<Message>,
    click: fn(LinkTarget) -> Message,
    hover: fn(Option<LinkTarget>) -> Message,
}

impl<Message> Clone for LinkHandlers<Message> {
    fn clone(&self) -> Self {
        LinkHandlers {
            input: self.input.clone(),
            click: self.click,
            hover: self.hover,
        }
    }
}

impl<Message: 'static> LinkHandlers<Message> {
    /// Links that send `click(target)` when clicked and `hover(Some(target))`
    /// / `hover(None)` as the pointer enters and leaves, to `input`.
    pub(super) fn new(
        input: relm4::Sender<Message>,
        click: fn(LinkTarget) -> Message,
        hover: fn(Option<LinkTarget>) -> Message,
    ) -> LinkHandlers<Message> {
        LinkHandlers {
            input,
            click,
            hover,
        }
    }

    /// A link button for `link`.
    pub(super) fn link(&self, link: &NameLink) -> gtk::Button {
        // A name is never cut short while it could be read whole: it
        // asks for its whole width (names are monospace, so that many
        // characters), and only a name longer than LINK_WHOLE_CHARS may
        // end in "…" in a narrow panel. Asking for less let the row hand
        // a short name a few pixels too few, and "foot_mid" showed as
        // "foot_mi…" with the panel half empty.
        let whole = i32::try_from(link.name.chars().count())
            .unwrap_or(LINK_WHOLE_CHARS)
            .min(LINK_WHOLE_CHARS);
        let label = gtk::Label::builder()
            .label(&link.name)
            .width_chars(whole)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let button = gtk::Button::builder()
            .child(&label)
            .css_classes(["stave-name-link"])
            .valign(gtk::Align::Center)
            .tooltip_text(match link.target {
                LinkTarget::Select(_) => fill_in(&tr("Select {name}"), &[("name", &link.name)]),
                LinkTarget::Space(_) => fill_in(&tr("The space {name}"), &[("name", &link.name)]),
            })
            .build();
        button.set_cursor_from_name(Some("pointer"));
        button.update_property(&[gtk::accessible::Property::Label(&link.name)]);
        // Sending fails only once the owner has gone, when nobody needs
        // to hear (docs/relm4-migration.md, rule 4).
        let (input, click, target) = (self.input.clone(), self.click, link.target.clone());
        button.connect_clicked(move |_| {
            let _ = input.send(click(target.clone()));
        });
        let motion = gtk::EventControllerMotion::new();
        let (input, hover, target) = (self.input.clone(), self.hover, link.target.clone());
        motion.connect_enter(move |_, _, _| {
            let _ = input.send(hover(Some(target.clone())));
        });
        let (input, hover) = (self.input.clone(), self.hover);
        motion.connect_leave(move |_| {
            let _ = input.send(hover(None));
        });
        button.add_controller(motion);
        button
    }

    /// The links in a row, wrapping onto more lines when they don't fit.
    pub(super) fn links(&self, links: &[NameLink]) -> adw::WrapBox {
        let wrap = adw::WrapBox::builder()
            .child_spacing(4)
            .line_spacing(3)
            .hexpand(true)
            .valign(gtk::Align::Center)
            .css_classes(["stave-name-links"])
            .build();
        for link in links {
            wrap.append(&self.link(link));
        }
        wrap
    }
}

/// The most characters of a name link always shown whole; a longer name
/// may end in "…" when the panel is too narrow for it.
const LINK_WHOLE_CHARS: i32 = 24;

/// A collapsible section: its title (and a grey note on the right) on a
/// faint band, and the box its rows go in. Open unless `open` is false.
pub(super) fn section(title: &str, note: &str, open: bool) -> (gtk::Box, gtk::Box) {
    let (section, body, note_label) = section_noted(title, open);
    note_label.set_label(note);
    note_label.set_visible(!note.is_empty());
    (section, body)
}

/// A collapsible section whose note is set later: the section, its body
/// and the note's label (hidden until it has words).
pub(super) fn section_noted(title: &str, open: bool) -> (gtk::Box, gtk::Box, gtk::Label) {
    let (
        SectionHead {
            button,
            revealer,
            body,
        },
        note,
    ) = section_head(title, open);
    revealer.set_child(Some(&body));
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-section"])
        .build();
    section.append(&button);
    section.append(&revealer);
    (section, body, note)
}

/// The widgets of a collapsible section, from [`section_head`].
pub(super) struct SectionHead {
    /// The head: a button with the arrow, the title and the note.
    pub(super) button: gtk::Button,
    /// What the head opens and closes; its child is to be `body`.
    pub(super) revealer: gtk::Revealer,
    /// The body (class `stave-section-body`), hidden while folded too:
    /// the caller sets it as the revealer's child and fills it.
    pub(super) body: gtk::Box,
}

/// The parts of a collapsible section, for a component that builds the
/// section in its own view, and the note's label (hidden until it has
/// words). The caller puts the head and the revealer, in that order, in
/// a vertical box with the class `stave-section`, and the body in the
/// revealer.
pub(super) fn section_head(title: &str, open: bool) -> (SectionHead, gtk::Label) {
    let head = gtk::Box::builder().spacing(6).build();
    let arrow = gtk::Image::builder()
        .icon_name(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        })
        .pixel_size(12)
        .css_classes(["stave-section-arrow"])
        .build();
    head.append(&arrow);
    head.append(
        &gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["stave-section-title"])
            .build(),
    );
    let note = gtk::Label::builder()
        .xalign(1.0)
        .visible(false)
        .css_classes(["stave-section-note"])
        .build();
    head.append(&note);
    let button = gtk::Button::builder()
        .child(&head)
        .css_classes(["flat", "stave-section-head"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(title)]);
    button.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
    // Hidden while folded as well: a revealer with no transition keeps
    // its child's height.
    let body = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-section-body"])
        .visible(open)
        .build();
    let revealer = gtk::Revealer::builder()
        .reveal_child(open)
        .transition_type(gtk::RevealerTransitionType::None)
        .build();
    {
        let revealer = revealer.clone();
        let body = body.clone();
        button.connect_clicked(move |button| {
            let open = !revealer.reveals_child();
            body.set_visible(open);
            revealer.set_reveal_child(open);
            arrow.set_icon_name(Some(if open {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            }));
            button.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
        });
    }
    (
        SectionHead {
            button,
            revealer,
            body,
        },
        note,
    )
}

/// A label | value row: the label in the grey column, then `value`.
pub(super) fn row(label: &str, value: &impl IsA<gtk::Widget>) -> gtk::Box {
    let line = gtk::Box::builder()
        .spacing(6)
        .css_classes(["stave-field-row"])
        .build();
    line.append(&row_label(label));
    value.set_hexpand(true);
    line.append(value);
    line
}

/// The grey label of a row, cut short with "…" (in full in its tooltip).
pub(super) fn row_label(label: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .valign(gtk::Align::Center)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .tooltip_text(label)
        .css_classes(["stave-field-label"])
        .build()
}

/// A value in its dark rounded field, wrapped when long.
pub(super) fn value(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(false)
        .css_classes(["stave-field-value"])
        .build()
}

/// A plain line of words inside a section (a note, a sentence).
pub(super) fn words(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(classes.to_vec())
        .build()
}

/// The header row: the swatch (if any), the name in a monospace field,
/// and on the right whatever `trailing` holds (the file:line link).
pub(super) fn header(
    swatch: Option<&gtk::Widget>,
    name: &impl IsA<gtk::Widget>,
    trailing: Option<&gtk::Widget>,
) -> gtk::Box {
    let line = gtk::Box::builder()
        .spacing(6)
        .css_classes(["stave-inspector-header"])
        .build();
    if let Some(swatch) = swatch {
        swatch.set_valign(gtk::Align::Center);
        line.append(swatch);
    }
    name.set_hexpand(true);
    line.append(name);
    if let Some(trailing) = trailing {
        trailing.set_valign(gtk::Align::Center);
        line.append(trailing);
    }
    line
}

/// The name in the header: a monospace field.
pub(super) fn name_field(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .tooltip_text(text)
        .css_classes(["stave-name-field"])
        .build()
}

/// The footer of an edit: Cancel (Esc) and the commit button, which always
/// reads "Apply" (Enter), at the right.
pub(super) fn footer_buttons() -> (gtk::Box, gtk::Button, gtk::Button) {
    let cancel = gtk::Button::builder()
        .label(tr("Cancel"))
        .tooltip_text(tr("Leave the design as it is (Esc)"))
        .css_classes(["stave-footer-button"])
        .build();
    cancel.set_child(Some(&key_hint(&tr("Cancel"), &tr("Esc"))));
    let apply = gtk::Button::builder()
        .css_classes(["suggested-action", "stave-footer-button"])
        .build();
    apply.set_child(Some(&key_hint(&tr(APPLY), &tr("Enter"))));
    apply.update_property(&[gtk::accessible::Property::Label(&tr(APPLY))]);
    cancel.update_property(&[gtk::accessible::Property::Label(&tr("Cancel"))]);
    let buttons = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::End)
        .build();
    buttons.append(&cancel);
    buttons.append(&apply);
    (buttons, cancel, apply)
}

/// Gives a footer button other words (its key hint stays): the hold
/// card's "Try it", which checks a rule before the change's card offers
/// Apply.
pub(super) fn set_button_words(button: &gtk::Button, words: &str) {
    if let Some(label) = button
        .child()
        .and_then(|child| child.first_child())
        .and_downcast::<gtk::Label>()
    {
        label.set_label(words);
    }
    button.update_property(&[gtk::accessible::Property::Label(words)]);
}

/// What every commit button says (translated where it is shown).
pub(super) const APPLY: &str = tr_noop("Apply");

/// A button's words and its key, dimmed: "Apply  Enter".
fn key_hint(words: &str, key: &str) -> gtk::Box {
    let content = gtk::Box::builder().spacing(6).build();
    content.append(&gtk::Label::new(Some(words)));
    content.append(
        &gtk::Label::builder()
            .label(key)
            .css_classes(["stave-key-hint"])
            .build(),
    );
    content
}

/// The words a footer button shows (for tests): its first label.
#[cfg(test)]
pub(super) fn button_words(button: &gtk::Button) -> Option<String> {
    button
        .child()
        .and_then(|child| child.first_child())
        .and_downcast::<gtk::Label>()
        .map(|label| label.label().to_string())
}

/// The look of the sections, rows and links, in libadwaita's named
/// colours and the design's link blue (#99c1f1).
const STYLE: &str = r#"
/* ---- lane: Inspector and editing (sections.rs) ---- */
.stave-inspector-compact { padding: 0 0 8px 0; }
.stave-inspector-compact .stave-code { margin: 2px 0 4px 0; font-size: 0.784rem; }
.stave-inspector-header { padding: 6px 8px 6px 8px; min-height: 26px; }
.stave-section.empty .stave-section-arrow { opacity: 0; }
button.stave-row-lead-button { padding: 0; min-height: 20px; }
button.stave-row-lead-button:hover .stave-field-label { color: currentColor; }
button.stave-check-row { padding: 2px 4px; font-weight: normal; }
.stave-name-field { font-family: var(--monospace-font-family, monospace); font-size: 0.852rem; padding: 2px 8px; min-height: 20px;
  border-radius: 4px; background: alpha(black, 0.22); }
.stave-source-link { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; padding: 0 4px; min-height: 20px; }
.stave-unsaved-dot { color: #f6d32d; font-size: 0.75rem; }
.stave-section { margin-top: 1px; }
button.stave-section-head { padding: 2px 8px 2px 4px; min-height: 22px; border-radius: 0;
  background: alpha(currentColor, 0.06); font-weight: normal; }
button.stave-section-head:hover { background: alpha(currentColor, 0.09); }
.stave-section-arrow { opacity: 0.7; }
.stave-section-title { font-weight: 700; font-size: 0.818rem; }
.stave-section-note { font-size: 0.784rem; color: alpha(currentColor, 0.55); }
.stave-section-body { padding: 3px 8px 6px 8px; }
.stave-field-row { min-height: 22px; padding: 1px 0; }
.stave-field-label { min-width: 104px; font-size: 0.818rem; color: alpha(currentColor, 0.6); }
.stave-field-value { font-size: 0.818rem; padding: 2px 6px; border-radius: 4px; background: alpha(black, 0.18); }
.stave-field-row entry { min-height: 22px; font-size: 0.818rem; padding-top: 0; padding-bottom: 0; }
button.stave-name-link { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; color: #99c1f1; padding: 0 6px;
  min-height: 18px; border-radius: 4px; background: alpha(#99c1f1, 0.12); box-shadow: none; }
button.stave-name-link:hover { background: alpha(#99c1f1, 0.24); }
.stave-name-links > flowboxchild { padding: 0; min-height: 0; }
.stave-row-lead { font-size: 0.818rem; }
.stave-row-lead.joint { font-family: var(--monospace-font-family, monospace); }
.stave-row-note { font-size: 0.784rem; color: alpha(currentColor, 0.55); }
.stave-check-row { font-size: 0.818rem; }
.stave-check-row.error { color: @error_color; }
.stave-check-row.warning { color: @warning_color; }
.stave-verdict-row { font-size: 0.818rem; font-weight: 700; }
.stave-verdict-row.fits { color: @success_color; }
.stave-verdict-row.refused { color: @error_color; }
.stave-diff { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; padding: 4px 6px; border-radius: 4px;
  background: alpha(black, 0.18); }
.stave-diff-place { color: alpha(currentColor, 0.55); }
.stave-diff-added { color: #8ff0a4; }
.stave-diff-removed { color: #ffa348; }
.stave-key-hint { font-size: 0.75rem; font-weight: normal; opacity: 0.6; }
button.stave-footer-button { font-size: 0.818rem; font-weight: 600; min-height: 24px; padding: 0 10px; }
.stave-edit-footer { padding: 6px 8px 8px 8px; border-top: 1px solid alpha(currentColor, 0.1); }
.stave-radio-row { min-height: 22px; font-size: 0.818rem; }
.stave-radio-row .stave-radio-name { font-family: var(--monospace-font-family, monospace); color: #99c1f1; }
.stave-radio-row .stave-radio-value { color: alpha(currentColor, 0.75); }
.stave-grip-readout { padding: 3px 10px; border-radius: 6px; background: alpha(@window_bg_color, 0.92);
  font-size: 0.818rem; box-shadow: 0 1px 3px alpha(black, 0.3); }
.stave-grip-readout .stave-readout-name { font-family: var(--monospace-font-family, monospace); color: #99c1f1; }
.stave-grip-readout .stave-readout-value { font-weight: 600; }
"#;

/// Puts the sections' stylesheet on the default display, once for the
/// process (Stave uses only the default display).
pub(super) fn install_style() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    let Some(display) = gdk::Display::default() else {
        return;
    };
    INSTALLED.call_once(|| add_style(&display));
}

/// Adds the sections' stylesheet to `display`.
fn add_style(display: &gdk::Display) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3,
    );
}

#[cfg(test)]
mod tests {
    use stave_app::session::{ProblemIndex, Selection};

    use super::*;
    use crate::ui::gtk_test::on_gtk_thread;

    /// What a test's links send.
    #[derive(Debug, PartialEq, Eq)]
    enum Heard {
        Click(LinkTarget),
        Hover(Option<LinkTarget>),
    }

    #[test]
    fn a_link_sends_its_owner_messages() {
        on_gtk_thread(links_send_messages);
    }

    /// A link's click and its pointer entering and leaving reach the
    /// owner's input as the owner's own messages, in order; once the
    /// owner's input has gone, a click is let go without a panic.
    fn links_send_messages() {
        let (input, heard) = relm4::channel::<Heard>();
        let handlers = LinkHandlers::new(input, Heard::Click, Heard::Hover);
        let target = LinkTarget::Select(Selection::Problem(ProblemIndex(0)));
        let link = handlers.link(&NameLink {
            name: "door_l".to_owned(),
            target: target.clone(),
        });
        link.emit_clicked();
        let motion = link
            .observe_controllers()
            .into_iter()
            .filter_map(Result::ok)
            .find_map(|controller| controller.downcast::<gtk::EventControllerMotion>().ok())
            .expect("the link watches the pointer");
        motion.emit_by_name::<()>("enter", &[&1.0_f64, &1.0_f64]);
        motion.emit_by_name::<()>("leave", &[]);
        // Three were sent, so none of these waits.
        let got: Vec<Heard> = (0..3).filter_map(|_| heard.recv_sync()).collect();
        assert_eq!(
            got,
            vec![
                Heard::Click(target.clone()),
                Heard::Hover(Some(target)),
                Heard::Hover(None),
            ]
        );
        drop(heard);
        drop(handlers);
        link.emit_clicked();
    }
}
