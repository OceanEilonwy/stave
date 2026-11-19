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
//!
//! It is a relm4 component, [`InspectorComponent`]: its model (the page
//! showing, the note for when there is nothing to select, the files with
//! unsaved changes) is the only state, only its update changes it, and
//! the buttons it draws only send [`InspectorInput`] messages.
//! [`Inspector`] keeps the older callback API in front of it until the
//! window moves to relm4 too.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::empty::Note;
use stave_app::focus::Target;
use stave_app::i18n::{fill_in, ntr, tr};
use stave_app::inspect::{
    Connection, ConnectionSection, Field, Inspection, LinkTarget, Snippet, Swatch, SwatchShape,
    TokenClass,
};
use stave_app::session::{Selection, SourceLocation};
use stave_lang::Severity;

use super::focus_panel::FocusPanel;
use super::icons;
use super::relm4_shim::Bridge;
use super::sections::{self, LinkHandlers};
use super::style::CODE;

/// Which of the inspector's pages shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// "Nothing selected".
    Empty,
    /// Why there is nothing to select.
    Note,
    /// The selection's details.
    Details,
    /// Focus mode's panel.
    Focus,
}

impl Page {
    /// The page's name in the stack.
    fn name(self) -> &'static str {
        match self {
            Page::Empty => "empty",
            Page::Note => "note",
            Page::Details => "details",
            Page::Focus => "focus",
        }
    }

    /// The page for nothing selected: the note when there is one.
    fn idle(note: Option<&Note>) -> Page {
        match note {
            Some(_) => Page::Note,
            None => Page::Empty,
        }
    }
}

/// The inspector's state.
pub struct InspectorComponent {
    /// Where the name links it draws report clicks and hovers. The links
    /// are built by `sections`, which isn't a component yet.
    links: LinkHandlers,
    /// The page showing.
    page: Page,
    /// What to say when there is nothing to select.
    idle: Option<Note>,
    /// The files with changes that aren't saved, for the ● after the
    /// file:line link. It is drawn when an inspection is shown.
    unsaved: BTreeSet<PathBuf>,
}

/// What an inspector is made with.
pub struct InspectorInit {
    /// Focus mode's panel, shown on the focus page.
    pub focus: gtk::Box,
    /// Where the name links it draws report clicks and hovers.
    pub links: LinkHandlers,
}

/// What the inspector is told.
#[derive(Debug)]
pub enum InspectorInput {
    /// Show focus mode's panel (it fills itself).
    ShowFocus,
    /// Say this when nothing is selected because there is nothing to
    /// select (`None`: the usual "Nothing selected" hint).
    ShowIdle(Option<Note>),
    /// Show an inspection, or the hint when nothing is selected.
    Show(Option<Box<Inspection>>),
    /// The files with changes that aren't saved, sent by the window when
    /// they change; the ● is drawn from them when an inspection is shown.
    Unsaved(BTreeSet<PathBuf>),
    /// People asked to see a joint or hardware in focus mode.
    FocusAsked(Target),
    /// People clicked the file:line link.
    SourceClicked(SourceLocation),
    /// People clicked something that selects or lights a target.
    Click(LinkTarget),
}

/// What the inspector tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectorOutput {
    /// Show this joint or hardware in focus mode.
    Focus(Target),
    /// Show this place in the editor.
    Source(SourceLocation),
    /// Select or light this target, as a link does.
    Link(LinkTarget),
}

#[relm4::component(pub)]
impl Component for InspectorComponent {
    type Init = InspectorInit;
    type Input = InspectorInput;
    type Output = InspectorOutput;
    type CommandOutput = ();

    view! {
        gtk::Stack {
            add_css_class: "stave-inspector-panel",
            set_width_request: 240,
            set_vhomogeneous: false,
            set_interpolate_size: true,

            add_named[Some(Page::Empty.name())] = &adw::StatusPage {
                set_title: &tr("Nothing selected"),
                set_description: Some(&tr("Click a part in the list or in the 3D view to see its stock, size, joints and where it is written.")),
                add_css_class: "compact",
            },
            add_named[Some(Page::Details.name())] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                set_max_content_height: 640,

                #[name = "details"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 0,
                    add_css_class: "stave-inspector-compact",
                },
            },
            add_named[Some(Page::Focus.name())] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                set_max_content_height: 640,

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    add_css_class: "stave-inspector",
                    append: &focus,
                },
            },
            add_named[Some(Page::Note.name())] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,
                set_max_content_height: 640,

                #[name = "note"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    add_css_class: "stave-inspector",
                },
            },

            #[watch]
            set_visible_child_name: model.page.name(),
        }
    }

    fn init(
        init: Self::Init,
        root: Self::Root,
        _sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        sections::install_style();
        let InspectorInit { focus, links } = init;
        let model = InspectorComponent {
            links,
            page: Page::Empty,
            idle: None,
            unsaved: BTreeSet::new(),
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: InspectorInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        let output = match message {
            InspectorInput::ShowFocus => {
                self.page = Page::Focus;
                None
            }
            InspectorInput::ShowIdle(note) => {
                if let Some(note) = &note {
                    fill_note(&widgets.note, note);
                }
                // Only a page saying nothing is selected changes.
                if matches!(self.page, Page::Empty | Page::Note) {
                    self.page = Page::idle(note.as_ref());
                }
                self.idle = note;
                None
            }
            InspectorInput::Show(None) => {
                self.page = Page::idle(self.idle.as_ref());
                None
            }
            InspectorInput::Show(Some(inspection)) => {
                let drawer = Drawer {
                    links: &self.links,
                    sender: &sender,
                    unsaved: &self.unsaved,
                };
                drawer.fill_details(&widgets.details, &inspection);
                self.page = Page::Details;
                None
            }
            InspectorInput::Unsaved(unsaved) => {
                self.unsaved = unsaved;
                None
            }
            InspectorInput::FocusAsked(target) => Some(InspectorOutput::Focus(target)),
            InspectorInput::SourceClicked(location) => Some(InspectorOutput::Source(location)),
            InspectorInput::Click(target) => Some(InspectorOutput::Link(target)),
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake: the owner may not want
            // to hear.
            let _ = sender.output(output);
        }
        self.update_view(widgets, sender);
    }
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

/// Fills the note page: why there is nothing to select.
fn fill_note(container: &gtk::Box, note: &Note) {
    clear(container);
    let head = gtk::Box::new(gtk::Orientation::Vertical, 6);
    head.append(&label(&note.title, &["stave-inspector-title"]));
    if !note.description.is_empty() {
        head.append(&label(&note.description, &["stave-description"]));
    }
    container.append(&head);
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
        container.append(&block);
    }
    if let Some(after) = &note.after {
        container.append(&label(after, &["stave-description"]));
    }
}

/// Draws an inspection's details. Its buttons only send the inspector
/// messages; its name links report to the link handlers.
struct Drawer<'a> {
    links: &'a LinkHandlers,
    sender: &'a ComponentSender<InspectorComponent>,
    unsaved: &'a BTreeSet<PathBuf>,
}

impl Drawer<'_> {
    /// Sends `message` to the inspector when `button` is clicked.
    fn on_click(&self, button: &gtk::Button, message: impl Fn() -> InspectorInput + 'static) {
        let sender = self.sender.input_sender().clone();
        button.connect_clicked(move |_| sender.emit(message()));
    }

    /// Fills `details` with the inspection: header, what it is, what it
    /// is joined to, its cuts, how it works, its checks and where it is
    /// written.
    fn fill_details(&self, details: &gtk::Box, inspection: &Inspection) {
        clear(details);
        details.append(&self.header(inspection));

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
        details.append(&section_box);

        let joined = connections_in(inspection, ConnectionSection::Joined);
        if !joined.is_empty() {
            let (section_box, body) =
                sections::section(&tr("Joined to"), &joined.len().to_string(), true);
            for connection in joined {
                body.append(&self.connection_row(connection));
            }
            details.append(&section_box);
        }
        let cuts = connections_in(inspection, ConnectionSection::Cuts);
        if !cuts.is_empty() {
            let (section_box, body) = sections::section(&tr("Cuts"), &cuts.len().to_string(), true);
            for connection in cuts {
                body.append(&self.connection_row(connection));
            }
            details.append(&section_box);
        }
        let how = connections_in(inspection, ConnectionSection::HowItWorks);
        if !how.is_empty() {
            let (section_box, body) =
                sections::section(&tr(inspection.connections_title), "", true);
            for connection in how {
                body.append(&self.connection_row(connection));
            }
            details.append(&section_box);
        }
        details.append(&self.checks_section(inspection));
        if let Some(source) = &inspection.source {
            details.append(&source_section(source));
        }
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
            if self.unsaved.contains(Path::new(&source.location.path)) {
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
        self.on_click(&link, move || {
            InspectorInput::SourceClicked(location.clone())
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
                self.on_click(&button, move || InspectorInput::FocusAsked(target));
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
            let problem = check.problem;
            self.on_click(&button, move || {
                InspectorInput::Click(LinkTarget::Select(Selection::Problem(problem)))
            });
            body.append(&button);
        }
        section_box
    }
}

/// The inspection's connections in one section, in order.
fn connections_in(inspection: &Inspection, section: ConnectionSection) -> Vec<&Connection> {
    inspection
        .connections
        .iter()
        .filter(|connection| connection.section == section)
        .collect()
}

/// "Defined in": the declaration, highlighted.
fn source_section(source: &Snippet) -> gtk::Box {
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

/// The inspector panel: it owns the component (dropping it shuts the
/// component down), passes its setters on as [`InspectorInput`] messages
/// and its [`InspectorOutput`]s to the callback connected with
/// [`Inspector::connect_outputs`].
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned.
pub struct Inspector {
    /// The panel, ready to put in the split view.
    pub root: gtk::Stack,
    /// Focus mode's panel.
    pub focus: FocusPanel,
    /// Where the links it draws report clicks and hovers.
    pub links: LinkHandlers,
    bridge: Bridge<InspectorComponent>,
}

impl Inspector {
    /// An empty inspector.
    pub fn new() -> Inspector {
        let focus = FocusPanel::new();
        let links = LinkHandlers::default();
        let bridge = Bridge::<InspectorComponent>::launch(InspectorInit {
            focus: focus.root.clone(),
            links: links.clone(),
        });
        Inspector {
            root: bridge.widget().clone(),
            focus,
            links,
            bridge,
        }
    }

    /// Calls `callback` with each of people's clicks from now on (a row
    /// asking for focus mode, the file:line link, a problem's link),
    /// instead of the one connected before.
    pub fn connect_outputs(&self, callback: impl Fn(InspectorOutput) + 'static) {
        self.bridge.connect(callback);
    }

    /// The files with changes that aren't saved, for the ● after the
    /// file:line link of the inspections shown from now on.
    pub fn show_unsaved(&self, unsaved: BTreeSet<PathBuf>) {
        self.bridge.send(InspectorInput::Unsaved(unsaved));
    }

    /// Shows focus mode's panel (it fills itself).
    pub fn show_focus(&self) {
        self.bridge.send(InspectorInput::ShowFocus);
    }

    /// Sets what to say when nothing is selected because there is nothing
    /// to select (`None`: the usual "Nothing selected" hint), and shows it
    /// if nothing is shown.
    pub fn show_idle(&self, note: Option<Note>) {
        self.bridge.send(InspectorInput::ShowIdle(note));
    }

    /// Shows an inspection, or the hint when nothing is selected.
    pub fn show(&self, inspection: Option<&Inspection>) {
        self.bridge
            .send(InspectorInput::Show(inspection.cloned().map(Box::new)));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use stave_app::session::LineNumber;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    #[test]
    fn problems_are_counted_in_words() {
        assert_eq!(problems_note(0), "0 problems");
        assert_eq!(problems_note(1), "1 problem");
    }

    /// Every button under `widget`, depth first.
    fn buttons(widget: &gtk::Widget) -> Vec<gtk::Button> {
        let mut found = Vec::new();
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Ok(button) = current.clone().downcast::<gtk::Button>() {
                found.push(button);
            }
            found.extend(buttons(&current));
            child = current.next_sibling();
        }
        found
    }

    /// An inspection written at `doors.stave:9`, with nothing else in it.
    fn inspection() -> Option<Inspection> {
        let line = LineNumber::new(9)?;
        Some(Inspection {
            eyebrow: "Part · in door_l".to_owned(),
            kind: "Part".to_owned(),
            within: Some("door_l".to_owned()),
            title: "stile".to_owned(),
            description: None,
            swatch: None,
            rows: Vec::new(),
            connections_title: "Joints and cuts",
            connections: Vec::new(),
            moves_with: None,
            checks: Vec::new(),
            source: Some(Snippet {
                label: "doors.stave:9".to_owned(),
                location: SourceLocation {
                    path: "doors.stave".to_owned(),
                    line,
                },
                lines: Vec::new(),
                cut_short: false,
            }),
        })
    }

    #[test]
    fn a_dropped_inspector_is_freed() {
        on_gtk_thread(inspector_is_freed);
    }

    /// Dropping the inspector shuts its component down when the main
    /// loop next runs: the panel, the model (and its runtime) and the
    /// callbacks are all freed.
    fn inspector_is_freed() {
        let inspector = Inspector::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        inspector.connect_outputs(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        inspector.show(inspection().as_ref());
        let root = inspector.root.downgrade();
        let input = inspector.bridge.sender().clone();
        settle();
        assert!(root.upgrade().is_some(), "the panel lives while held");
        drop(inspector);
        settle();
        assert!(root.upgrade().is_none(), "the panel is freed");
        assert!(
            input.send(InspectorInput::ShowFocus).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callbacks are freed");
    }

    #[test]
    fn setters_choose_the_page_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    fn setters_do_not_call_back() {
        let inspector = Inspector::new();
        let calls = Rc::new(RefCell::new(0));
        let counted = Rc::clone(&calls);
        inspector.connect_outputs(move |_| *counted.borrow_mut() += 1);
        let page = |inspector: &Inspector| inspector.root.visible_child_name();

        inspector.show(inspection().as_ref());
        assert_eq!(page(&inspector).as_deref(), Some("empty"), "only queued");
        settle();
        assert_eq!(page(&inspector).as_deref(), Some("details"));
        inspector.show_focus();
        settle();
        assert_eq!(page(&inspector).as_deref(), Some("focus"));
        inspector.show(None);
        settle();
        assert_eq!(page(&inspector).as_deref(), Some("empty"));
        inspector.show_idle(Some(Note {
            title: "Nothing to select".to_owned(),
            description: String::new(),
            code: None,
            after: None,
        }));
        settle();
        assert_eq!(page(&inspector).as_deref(), Some("note"));
        assert_eq!(*calls.borrow(), 0, "setters never call back");
    }

    #[test]
    fn a_click_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(clicks_are_queued);
    }

    /// The old inspector called its callback from inside the click, with
    /// GTK's signal still on the stack. Now a click only queues a
    /// message; its output reaches the callback after the update has
    /// returned, and a click fired from inside the callback is queued
    /// too. The ● comes from the unsaved files the window sent.
    fn clicks_are_queued() {
        let inspector = Rc::new(Inspector::new());
        inspector.show_unsaved(BTreeSet::from([PathBuf::from("doors.stave")]));
        inspector.show(inspection().as_ref());
        settle();
        let details = inspector.root.upcast_ref::<gtk::Widget>().clone();
        let Some(link) = buttons(&details)
            .into_iter()
            .find(|button| button.label().as_deref() == Some("doors.stave:9"))
        else {
            panic!("the inspection has a file:line link");
        };
        assert!(
            inspector
                .bridge
                .model()
                .unsaved
                .contains(Path::new("doors.stave")),
            "the unsaved files the window sent are kept"
        );

        let heard: Rc<RefCell<Vec<(u32, bool)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&inspector);
        let inner = link.clone();
        inspector.connect_outputs(move |output| {
            let InspectorOutput::Source(location) = output else {
                return;
            };
            let Some(inspector) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            if first {
                // A click from in here is only queued, as is a setter.
                inner.emit_clicked();
                inspector.show_focus();
            }
            let focus = inspector.root.visible_child_name().as_deref() == Some("focus");
            log.borrow_mut().push((location.line.get(), focus));
        });

        link.emit_clicked();
        assert!(heard.borrow().is_empty(), "the click is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![(9, false), (9, true)],
            "each output after its update, the inner click after the first"
        );
    }
}
