//! Focus mode's side panel (JointFocus.dc.html, Hinge.dc.html): for a
//! joint, its name and description, Goes in / Receives it / the cut / Fit,
//! the X-ray, Tint cut faces and Fade other parts toggles, and its insert
//! steps as written with a link to the kind; for hardware, "How this
//! hinge works", its numbered members and Opens to / Closest to the side /
//! Swing check. Everything it shows comes from `stave_app::focus::Focus`;
//! this module only lays it out. The step player, the swing slider and the
//! small pictures are in the 3D view's area (`focus_stage`).
//!
//! The panel is a relm4 component, [`FocusPanelComponent`]: its model is
//! the only state, only its update changes it, and its widgets' handlers
//! only send it messages. [`FocusPanel`] owns it; the window hears
//! people's presses as [`FocusPanelOutput`]s.

use adw::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::focus::{self, Focus};
use stave_app::i18n::{fill_in, tr};
use stave_app::session::SourceLocation;

use super::icons::{self, PathIcon};
use super::style::CODE;

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

/// A toggle's check box, on, named for screen readers.
fn check(title: &str) -> gtk::CheckButton {
    let check = gtk::CheckButton::builder()
        .active(true)
        .valign(gtk::Align::Center)
        .build();
    check.update_property(&[gtk::accessible::Property::Label(title)]);
    check
}

/// A toggle row around `check`: bold title, dim words, the check box at
/// the end. Returns the row and its words.
fn toggle_row(title: &str, words: &str, check: &gtk::CheckButton) -> (gtk::Box, gtk::Label) {
    let text = gtk::Label::builder()
        .use_markup(true)
        .xalign(0.0)
        .hexpand(true)
        .wrap(true)
        .build();
    set_toggle_text(&text, title, words);
    let row = gtk::Box::builder()
        .spacing(10)
        .css_classes(["stave-card-row", "stave-toggle-row"])
        .build();
    row.append(&text);
    row.append(check);
    // Clicking anywhere on the row toggles it, as clicking the box would.
    let click = gtk::GestureClick::new();
    let target = check.downgrade();
    click.connect_released(move |_, _, _, _| {
        if let Some(check) = target.upgrade() {
            check.set_active(!check.is_active());
        }
    });
    text.add_controller(click);
    (row, text)
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

/// The focus panel's state: where its link to the joint's kind goes.
#[derive(Debug, Default)]
pub struct FocusPanelComponent {
    /// Where "how it goes together"'s link goes, if anywhere.
    insert_location: Option<SourceLocation>,
}

/// What the focus panel is made with: its close button and its checks.
#[derive(Debug)]
pub struct FocusPanelInit {
    /// Leaves focus mode.
    pub done: gtk::Button,
    /// Draws the receiving part see-through.
    pub xray: gtk::CheckButton,
    /// Tints the faces cuts made.
    pub tint: gtk::CheckButton,
    /// Fades the other parts.
    pub fade: gtk::CheckButton,
}

/// What the focus panel is told.
#[derive(Debug)]
pub enum FocusPanelInput {
    /// Show a focus, or hide the panel (`None`).
    Show(Option<Box<Focus>>),
    /// Show these options on the X-ray, Tint and Fade checks, without
    /// telling anyone.
    ShowOptions(focus::Options),
    /// People pressed the link to the joint's kind.
    LinkPressed,
    /// People pressed the close button.
    DonePressed,
    /// People turned X-ray on or off.
    XrayToggled(bool),
    /// People turned Tint cut faces on or off.
    TintToggled(bool),
    /// People turned Fade other parts on or off.
    FadeToggled(bool),
}

/// What the focus panel tells its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum FocusPanelOutput {
    /// People asked to see the joint's kind in the text.
    Source(SourceLocation),
    /// People asked to leave focus mode.
    Done,
    /// People turned X-ray on or off.
    Xray(bool),
    /// People turned Tint cut faces on or off.
    Tint(bool),
    /// People turned Fade other parts on or off.
    Fade(bool),
}

/// The widgets the panel fills in when it shows a focus.
#[derive(Debug)]
pub struct FocusPanelWidgets {
    eyebrow: gtk::Label,
    heading: gtk::Label,
    description: gtk::Label,
    more: gtk::Expander,
    more_text: gtk::Label,
    members: gtk::Box,
    rows: gtk::Box,
    show_section: gtk::Box,
    xray_label: gtk::Label,
    insert_section: gtk::Box,
    insert_link: gtk::Button,
    insert_code: gtk::Label,
    /// The X-ray, Tint and Fade checks with their handlers, blocked while
    /// `ShowOptions` sets them.
    checks: [(gtk::CheckButton, gtk::glib::SignalHandlerId); 3],
}

impl Component for FocusPanelComponent {
    type Init = FocusPanelInit;
    type Input = FocusPanelInput;
    type Output = FocusPanelOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = FocusPanelWidgets;

    fn init_root() -> gtk::Box {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
        root.set_visible(false);
        root
    }

    fn init(
        init: FocusPanelInit,
        root: gtk::Box,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let FocusPanelInit {
            done,
            xray,
            tint,
            fade,
        } = init;
        let eyebrow = label(&["stave-eyebrow", "stave-eyebrow-bold", "stave-caps"]);
        eyebrow.set_hexpand(true);
        let input = sender.input_sender().clone();
        done.connect_clicked(move |_| input.emit(FocusPanelInput::DonePressed));
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
            .label(tr("More"))
            .child(&more_text)
            .css_classes(["stave-more"])
            .visible(false)
            .build();
        head.append(&more);

        let members = card();
        let rows = card();

        let (xray_row, xray_label) = toggle_row(&tr("X-ray"), &tr("see into the part"), &xray);
        let (tint_row, _) = toggle_row(&tr("Tint cut faces"), &tr("holes, mortises, dados"), &tint);
        let (fade_row, _) = toggle_row(&tr("Fade other parts"), "", &fade);
        let checks = [
            (
                &xray,
                FocusPanelInput::XrayToggled as fn(bool) -> FocusPanelInput,
            ),
            (&tint, FocusPanelInput::TintToggled),
            (&fade, FocusPanelInput::FadeToggled),
        ]
        .map(|(check, toggled)| {
            let input = sender.input_sender().clone();
            let handler =
                check.connect_active_notify(move |check| input.emit(toggled(check.is_active())));
            (check.clone(), handler)
        });
        let toggles = card();
        toggles.append(&xray_row);
        toggles.append(&tint_row);
        toggles.append(&fade_row);
        let show_title = label(&["stave-section", "stave-caps"]);
        show_title.set_label(&tr("Show"));
        let show_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        show_section.append(&show_title);
        show_section.append(&toggles);

        let insert_link = super::style::as_link(
            gtk::Button::builder()
                .css_classes(["flat", "stave-link"])
                .tooltip_text(tr("Show the joint's kind in the text"))
                .valign(gtk::Align::Center)
                .build(),
        );
        let input = sender.input_sender().clone();
        insert_link.connect_clicked(move |_| input.emit(FocusPanelInput::LinkPressed));
        let insert_head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let insert_title = label(&["stave-section", "stave-caps"]);
        insert_title.set_label(&tr("How it goes together"));
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
        insert_note.set_label(&tr("The animation plays the joint’s own insert steps, the same ones the assembly check runs backwards. What you see is what was checked."));
        let insert_section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        insert_section.append(&insert_head);
        insert_section.append(&insert_code);
        insert_section.append(&insert_note);

        root.append(&head);
        root.append(&members);
        root.append(&rows);
        root.append(&show_section);
        root.append(&insert_section);

        let widgets = FocusPanelWidgets {
            eyebrow,
            heading,
            description,
            more,
            more_text,
            members,
            rows,
            show_section,
            xray_label,
            insert_section,
            insert_link,
            insert_code,
            checks,
        };
        ComponentParts {
            model: FocusPanelComponent::default(),
            widgets,
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut FocusPanelWidgets,
        message: FocusPanelInput,
        sender: ComponentSender<Self>,
        root: &gtk::Box,
    ) {
        let output = match message {
            FocusPanelInput::Show(focus) => {
                self.insert_location = focus
                    .as_ref()
                    .and_then(|focus| focus.insert.as_ref())
                    .and_then(|insert| insert.location.clone());
                show(widgets, root, focus.as_deref());
                None
            }
            FocusPanelInput::ShowOptions(options) => {
                let [xray, tint, fade] = &widgets.checks;
                for ((check, handler), on) in [
                    (xray, options.xray),
                    (tint, options.tint_cut_faces),
                    (fade, options.fade_others),
                ] {
                    // What `#[block_signal]` does: the window's setting is
                    // not people's change.
                    check.block_signal(handler);
                    check.set_active(on);
                    check.unblock_signal(handler);
                }
                None
            }
            FocusPanelInput::LinkPressed => {
                self.insert_location.clone().map(FocusPanelOutput::Source)
            }
            FocusPanelInput::DonePressed => Some(FocusPanelOutput::Done),
            FocusPanelInput::XrayToggled(on) => Some(FocusPanelOutput::Xray(on)),
            FocusPanelInput::TintToggled(on) => Some(FocusPanelOutput::Tint(on)),
            FocusPanelInput::FadeToggled(on) => Some(FocusPanelOutput::Fade(on)),
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake.
            let _ = sender.output(output);
        }
    }
}

/// Lays a focus out in the panel, or hides it (`None`).
fn show(widgets: &FocusPanelWidgets, root: &gtk::Box, focus: Option<&Focus>) {
    clear(&widgets.members);
    clear(&widgets.rows);
    let Some(focus) = focus else {
        root.set_visible(false);
        return;
    };
    root.set_visible(true);
    let joint = focus.assembles();
    widgets
        .eyebrow
        .set_label(&if joint { tr("Joint") } else { tr("Hardware") });
    widgets.heading.set_label(&focus.heading);
    // The first paragraph (two sentences at most), the rest under More.
    let full = focus.description.as_deref().unwrap_or(&focus.title);
    let summary = focus.summary.as_deref().unwrap_or(full);
    widgets.description.set_label(summary);
    widgets.description.set_tooltip_text(Some(&focus.title));
    let rest = full.strip_prefix(summary).unwrap_or(full).trim();
    widgets.more_text.set_label(rest);
    widgets.more.set_expanded(false);
    widgets
        .more
        .set_visible(!rest.is_empty() && summary != full);

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
        widgets.members.append(&line);
    }
    widgets.members.set_visible(!focus.members.is_empty());

    // Goes in, Receives it, the cut, Fit; or Opens to, Closest, Swing check.
    for entry in &focus.rows {
        let line = gtk::Box::builder()
            .spacing(12)
            .css_classes(["stave-card-row"])
            .build();
        line.append(
            &gtk::Label::builder()
                .label(tr(entry.label))
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
            let good = entry.value == focus::fits_words() || entry.value == focus::clear_words();
            value.add_css_class(if good { "good" } else { "bad" });
        }
        line.append(&value);
        widgets.rows.append(&line);
    }
    widgets.rows.set_visible(!focus.rows.is_empty());

    widgets.show_section.set_visible(joint);
    let see_into = match focus.receives.as_deref() {
        Some(receives) => fill_in(&tr("see into {part}"), &[("part", &receives)]),
        None => tr("see into the part"),
    };
    set_toggle_text(&widgets.xray_label, &tr("X-ray"), &see_into);

    match &focus.insert {
        Some(insert) => {
            widgets.insert_section.set_visible(true);
            widgets.insert_link.set_label(&insert.link);
            widgets.insert_link.set_visible(insert.location.is_some());
            widgets
                .insert_code
                .set_markup(&insert_markup(&insert.steps));
            widgets
                .insert_code
                .update_property(&[gtk::accessible::Property::Label(&insert.text)]);
        }
        None => widgets.insert_section.set_visible(false),
    }
}

/// Focus mode's side panel. It owns the component's controller: dropping
/// it shuts the component down. Setters are queued; people's presses go
/// to the owner as [`FocusPanelOutput`]s, forwarded at launch.
pub struct FocusPanel {
    /// The panel, ready to put in the side panel.
    pub root: gtk::Box,
    /// Leaves focus mode.
    #[cfg(test)]
    pub(in crate::ui) done: gtk::Button,
    /// Draws the receiving part see-through.
    #[cfg(test)]
    pub(in crate::ui) xray: gtk::CheckButton,
    /// Tints the faces cuts made.
    #[cfg(test)]
    pub(in crate::ui) tint: gtk::CheckButton,
    /// Fades the other parts.
    #[cfg(test)]
    pub(in crate::ui) fade: gtk::CheckButton,
    controller: Controller<FocusPanelComponent>,
}

impl FocusPanel {
    /// An empty, hidden panel, people's presses made the owner's messages
    /// by `map` and sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(FocusPanelOutput) -> X + 'static,
    ) -> FocusPanel {
        let done = gtk::Button::builder()
            .child(&PathIcon::new(icons::CLOSE, 12))
            .tooltip_text(tr("Leave focus mode (Esc)"))
            .css_classes(["flat", "circular", "stave-close"])
            .valign(gtk::Align::Center)
            .build();
        done.update_property(&[gtk::accessible::Property::Label(&tr("Leave focus mode"))]);
        let (xray, tint, fade) = (
            check(&tr("X-ray")),
            check(&tr("Tint cut faces")),
            check(&tr("Fade other parts")),
        );
        let controller = FocusPanelComponent::builder()
            .launch(FocusPanelInit {
                done: done.clone(),
                xray: xray.clone(),
                tint: tint.clone(),
                fade: fade.clone(),
            })
            .forward(to, map);
        FocusPanel {
            root: controller.widget().clone(),
            #[cfg(test)]
            done,
            #[cfg(test)]
            xray,
            #[cfg(test)]
            tint,
            #[cfg(test)]
            fade,
            controller,
        }
    }

    /// Queues `input`; once the panel has gone there is nothing to show.
    fn send(&self, input: FocusPanelInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Shows a focus, or hides the panel (`None`).
    pub fn show(&self, focus: Option<&Focus>) {
        self.send(FocusPanelInput::Show(focus.cloned().map(Box::new)));
    }

    /// Shows focus mode's options on the X-ray, Tint and Fade checks,
    /// without calling back.
    pub fn show_options(&self, options: focus::Options) {
        self.send(FocusPanelInput::ShowOptions(options));
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{listener, on_gtk_thread, recorder, settle};

    /// Focus on the stool's rung sockets, as the window opens it.
    fn socket_focus() -> Focus {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stool");
        let Ok(opened) = stave_app::open(&folder) else {
            panic!("the stool opens");
        };
        let Some(design) = opened.design.as_ref() else {
            panic!("the stool solves");
        };
        let scene = stave_app::view::scene(design);
        let Ok(target) = stave_app::focus::find(design, "rung_sockets", None) else {
            panic!("the stool has rung sockets");
        };
        let Some(focus) = stave_app::focus::open(&opened, design, &scene, target) else {
            panic!("focus opens on them");
        };
        focus
    }

    #[test]
    fn the_panel_shows_a_focus_and_its_link_goes_to_the_kind() {
        on_gtk_thread(panel_link);
    }

    /// The old panel kept the link's place in a shared cell read by the
    /// click handler; now the click is a message, the model knows the
    /// place, and the window hears it after the update.
    fn panel_link() {
        let shared: Rc<RefCell<Option<std::rc::Weak<FocusPanel>>>> = Rc::default();
        let heard: Rc<RefCell<Vec<SourceLocation>>> = Rc::default();
        let (log, weak) = (Rc::clone(&heard), Rc::clone(&shared));
        let to = listener(move |output| {
            let FocusPanelOutput::Source(location) = output else {
                return;
            };
            log.borrow_mut().push(location);
            if let Some(panel) = weak.borrow().as_ref().and_then(std::rc::Weak::upgrade) {
                // The window leaving focus from inside its handler.
                panel.show(None);
            }
        });
        let panel = Rc::new(FocusPanel::new(&to, |output| output));
        shared.replace(Some(Rc::downgrade(&panel)));
        let focus = socket_focus();
        let Some(location) = focus
            .insert
            .as_ref()
            .and_then(|insert| insert.location.clone())
        else {
            panic!("a socket's kind is written somewhere");
        };
        panel.show(Some(&focus));
        assert!(!panel.root.is_visible(), "show is queued");
        settle();
        assert!(panel.root.is_visible());
        let link = panel.controller.model().insert_location.clone();
        assert_eq!(link.as_ref(), Some(&location));
        // The link button is the last child of the insert section's head.
        let Some(button) = std::iter::successors(panel.root.first_child(), |widget| {
            widget.first_child().or_else(|| next_in_tree(widget))
        })
        .find_map(|widget| {
            widget
                .downcast::<gtk::Button>()
                .ok()
                .filter(|button| button.has_css_class("stave-link"))
        }) else {
            panic!("the panel has a link to the kind");
        };
        button.emit_clicked();
        assert!(heard.borrow().is_empty(), "the click is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![location]);
        assert!(!panel.root.is_visible(), "the window's show came after");
    }

    /// The next widget after `widget` in a depth-first walk, past its
    /// children.
    fn next_in_tree(widget: &gtk::Widget) -> Option<gtk::Widget> {
        let mut at = widget.clone();
        loop {
            if let Some(next) = at.next_sibling() {
                return Some(next);
            }
            at = at.parent()?;
        }
    }

    #[test]
    fn the_panels_controls_are_heard_after_the_update_and_it_is_freed() {
        on_gtk_thread(panel_controls);
    }

    fn panel_controls() {
        let heard: Rc<RefCell<Vec<FocusPanelOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let to = listener(move |output| {
            let _ = &held;
            log.borrow_mut().push(output);
        });
        let panel = FocusPanel::new(&to, |output| output);
        drop(to);
        let callback = Rc::downgrade(&token);
        drop(token);
        panel.done.emit_clicked();
        panel.xray.set_active(false);
        panel.tint.set_active(false);
        panel.fade.set_active(false);
        assert!(heard.borrow().is_empty(), "queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                FocusPanelOutput::Done,
                FocusPanelOutput::Xray(false),
                FocusPanelOutput::Tint(false),
                FocusPanelOutput::Fade(false),
            ]
        );
        let root = panel.root.downgrade();
        let done = panel.done.downgrade();
        let input = panel.controller.sender().clone();
        drop(panel);
        settle();
        assert!(root.upgrade().is_none(), "the panel is freed");
        assert!(done.upgrade().is_none(), "its controls are freed");
        assert!(input.send(FocusPanelInput::Show(None)).is_err());
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn showing_the_options_sets_the_checks_without_calling_back() {
        on_gtk_thread(show_options_sets_checks);
    }

    /// The window shows its options on the checks with their handlers
    /// blocked, so it never hears its own setting as people's change; the
    /// last of several in one turn wins.
    fn show_options_sets_checks() {
        let (to, heard) = recorder();
        let panel = FocusPanel::new(&to, |output| output);
        panel.xray.set_active(false);
        panel.fade.set_active(false);
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![FocusPanelOutput::Xray(false), FocusPanelOutput::Fade(false)]
        );
        let mut options = focus::Options::default();
        panel.show_options(options);
        options.tint_cut_faces = false;
        panel.show_options(options);
        assert!(!panel.xray.is_active(), "show_options is queued");
        settle();
        assert!(panel.xray.is_active() && !panel.tint.is_active() && panel.fade.is_active());
        assert_eq!(heard.borrow().len(), 2, "show_options never calls back");
    }
}
