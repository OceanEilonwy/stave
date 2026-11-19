//! The card for a change of the design (`docs/editing.md`, "Changes of
//! the design"): a repeat, the shares grip, a mirror, a held distance, a
//! removal, a rename. It draws what [`Session::change`] holds after each
//! [`EditEvent`] (what the change does, what it found, the verdict with the
//! checker's problems and fixes) and sends the session's commands for its
//! buttons. A removal that something still uses asks first, in a dialog
//! that names each thing that would go with it.
//!
//! It is drawn as the Inspector is (`super::super::sections`): a header
//! row with the item's name, then collapsible sections (the change, the
//! size a grip moves, the verdict, the lines Apply would write) and a
//! footer whose commit button always reads Apply; for a removal it keeps
//! a destructive look, and the change's own line says what goes.
//!
//! The card and the removal's dialog are relm4 components
//! ([`ChangeCardComponent`], [`DependantsComponent`]) following
//! `docs/relm4-migration.md`; [`ChangeCard`] and [`DependantsDialog`] keep
//! the older API in front of them while the window (`impl Window`) is old
//! code. The window shows and hides them at once; what they say is drawn
//! when the main loop next runs.
//!
//! ```text
//! ┌────────────────────────────────────────┐
//! │ [back                 ]         remove │
//! │ ▾ Change                               │
//! │   Remove back                          │
//! │   Also removes: drawer_bay, drawer     │
//! │ ▾ Verdict                              │
//! │   ✓ It fits                            │
//! │ ▾ Pending                  @ line 88   │
//! │        [Cancel  Esc] [Apply  Enter]    │
//! └────────────────────────────────────────┘
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::change::{Change, ChangeDetail, ChangeRequest};
use stave_app::edit::Verdict;
use stave_app::editing::{EditCommand, Progress};
use stave_app::remove::Dependants;

use super::super::Window;
use super::super::edit_card::diagnostic_head;
use super::super::relm4_shim::Bridge;
use super::super::sections;
use super::card_verdict;
use crate::ui::update::editing::EditingInput;

/// What the user did on a change's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum ChangeAction {
    /// Pressed the card's commit button (Apply).
    Add,
    /// Pressed Cancel (or Escape).
    Cancel,
    /// Pressed a fix's button.
    Fix {
        /// The problem, in the refusal's order.
        problem: usize,
        /// The fix, among the problem's.
        fix: usize,
    },
    /// A removal waiting on its dependants: remove them too.
    IncludeDependants,
}

impl ChangeAction {
    /// The session's command for it.
    pub(in crate::ui) fn command(self) -> EditCommand {
        match self {
            ChangeAction::Add => EditCommand::Add,
            ChangeAction::Cancel => EditCommand::Cancel,
            ChangeAction::Fix { problem, fix } => EditCommand::ApplyFix { problem, fix },
            ChangeAction::IncludeDependants => EditCommand::IncludeDependants,
        }
    }
}

/// How the commit button (always "Apply") looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Commit {
    /// It writes something in: the suggested look.
    Write,
    /// It takes something out: the destructive look.
    Remove,
}

/// What a change's card says, worked out from the change alone (so it is
/// tested without a window).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct ChangeWords {
    /// "CHANGE · REPEAT".
    pub(in crate::ui) eyebrow: String,
    /// The change's summary: "Make 3 of drawer".
    pub(in crate::ui) title: String,
    /// The item it changes, for the header: "back".
    pub(in crate::ui) item: Option<String>,
    /// What it found, a line each.
    pub(in crate::ui) lines: Vec<String>,
    /// The things a removal would take with it, by name, nearest first.
    pub(in crate::ui) dependants: Vec<String>,
    /// `true` while a removal waits for the user to say whether those go.
    pub(in crate::ui) waiting_on_dependants: bool,
    /// The verdict; `None` while something is still to choose.
    pub(in crate::ui) verdict: Option<Verdict>,
    /// What is still to choose, in words, while the verdict is `None`.
    pub(in crate::ui) needs: Option<String>,
    /// The commit button's look (it always reads "Apply").
    pub(in crate::ui) commit: Commit,
    /// Whether the commit button can be pressed.
    pub(in crate::ui) can_add: bool,
    /// For a mirror: the switch that started it, shown on (turning it off
    /// cancels).
    pub(in crate::ui) switch: Option<&'static str>,
}

/// The item a change acts on, as named in its request.
pub(in crate::ui) fn change_item(request: &ChangeRequest) -> Option<String> {
    match request {
        ChangeRequest::Repeat { item, .. }
        | ChangeRequest::Mirror { item }
        | ChangeRequest::MirrorSplit { item, .. }
        | ChangeRequest::SetParam { item, .. }
        | ChangeRequest::Remove { item }
        | ChangeRequest::Rename { item, .. } => Some(item.as_str().to_owned()),
        ChangeRequest::RepeatPart { part, .. } => Some(part.as_str().to_owned()),
        ChangeRequest::MoveShares { region, .. } => Some(region.as_str().to_owned()),
        ChangeRequest::ParamKind { param, .. } => Some(param.as_str().to_owned()),
        ChangeRequest::Grip { .. } | ChangeRequest::Hold(_) => None,
    }
}

/// The eyebrow's word for a change.
fn kind_word(request: &ChangeRequest) -> &'static str {
    match request {
        ChangeRequest::Repeat { .. } | ChangeRequest::RepeatPart { .. } => "repeat",
        ChangeRequest::MoveShares { .. } => "sizes of the pieces",
        ChangeRequest::Mirror { .. } | ChangeRequest::MirrorSplit { .. } => "the other side",
        ChangeRequest::SetParam { .. } | ChangeRequest::Grip { .. } => "a size",
        ChangeRequest::ParamKind { .. } => "a param",
        ChangeRequest::Hold(_) => "hold a distance",
        ChangeRequest::Remove { .. } => "remove",
        ChangeRequest::Rename { .. } => "rename",
    }
}

/// What a change's card says.
pub(in crate::ui) fn change_words(change: &Change, checking: bool) -> ChangeWords {
    let request = change.request();
    let mut lines = Vec::new();
    let mut dependants = Vec::new();
    let mut waiting_on_dependants = false;
    let mut commit = Commit::Write;
    match change.detail() {
        ChangeDetail::Repeat => {
            if let ChangeRequest::MoveShares {
                region,
                lower,
                boundary,
            } = request
            {
                lines.push(format!(
                    "Piece {} of {} becomes {}; the piece after it takes the rest.",
                    lower.saturating_add(1),
                    region.as_str(),
                    super::hold::length_text(*boundary)
                ));
                lines.push(
                    "Only these two pieces change size; the shares written are small whole numbers."
                        .to_owned(),
                );
            }
        }
        ChangeDetail::Mirror {
            renamed,
            image,
            about,
        } => {
            if let Some(item) = change_item(request)
                && item != renamed.as_str()
            {
                lines.push(format!("{item} is renamed {}", renamed.as_str()));
            }
            lines.push(format!(
                "Adds {}, its mirror image about {about}",
                image.as_str()
            ));
        }
        ChangeDetail::Hold { name, condition } => {
            lines.push(format!("Writes the rule {}:", name.as_str()));
            lines.push(condition.clone());
            if let ChangeRequest::Hold(request) = request {
                lines.push(format!("Its message, the reason: “{}”", request.reason));
            }
        }
        ChangeDetail::Remove {
            removal,
            dependants: taken,
        } => {
            commit = Commit::Remove;
            dependants = removal
                .dependants
                .iter()
                .map(|item| item.name.clone())
                .collect();
            match taken {
                Dependants::Refuse if !dependants.is_empty() => {
                    waiting_on_dependants = true;
                    lines.push(format!(
                        "{} is used by {}. It can only go if they go too.",
                        removal.item.name,
                        dependants.join(", ")
                    ));
                }
                Dependants::Refuse => {}
                Dependants::Included => {
                    lines.push(format!("Also removes: {}", dependants.join(", ")));
                }
            }
        }
        ChangeDetail::Rename { from, to } => {
            lines.push(format!(
                "{from} becomes {} everywhere it is used",
                to.as_str()
            ));
            let files: Vec<String> = change
                .renamed_files()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect();
            if !files.is_empty() {
                lines.push(format!("Rewrites {}", files.join(", ")));
            }
        }
        // The size it moves, what else changes and where it stops are the
        // grip's own section (`grip_panel.rs`).
        ChangeDetail::Grip(_) => {}
    }
    let progress = change.progress();
    let verdict = card_verdict(progress, checking);
    let needs = match progress {
        Progress::NeedsChoice(names) if waiting_on_dependants => Some(format!(
            "Choose whether {} go too",
            names
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Progress::NeedsChoice(names) => Some(format!(
            "Still to choose: {}",
            names
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Progress::Tried(_) | Progress::Unwritable(_) => None,
    };
    let can_add = !checking && matches!(progress, Progress::Tried(Verdict::Fits(_)));
    ChangeWords {
        eyebrow: format!("CHANGE · {}", kind_word(request).to_uppercase()),
        title: change.summary().to_owned(),
        item: change_item(request),
        lines,
        dependants,
        waiting_on_dependants,
        verdict,
        needs,
        commit,
        can_add,
        switch: matches!(request, ChangeRequest::Mirror { .. }).then_some("And the other side"),
    }
}

/// The card for a change of the design, as a relm4 component: its model
/// is whether Apply can be pressed (Enter presses it only then); what it
/// says is drawn from each [`ChangeWords`] it is shown, and its buttons,
/// switch and keys only send [`ChangeCardInput`] messages. Its owner shows
/// and hides the card.
#[derive(Debug, Default)]
pub(in crate::ui) struct ChangeCardComponent {
    /// Whether the commit button can be pressed.
    can_add: bool,
}

/// What a change's card is made with: the two slots its owner fills
/// (`grip_panel.rs`, the Pending section), placed in its body each time
/// it is drawn.
#[derive(Debug)]
pub(in crate::ui) struct ChangeCardInit {
    /// Where the grips put the size a drag moves.
    pub(in crate::ui) grip_slot: gtk::Box,
    /// Where the window puts the "Pending" section.
    pub(in crate::ui) pending_slot: gtk::Box,
}

/// A change card's widgets.
#[derive(Debug)]
pub(in crate::ui) struct ChangeCardWidgets {
    body: gtk::Box,
    add: gtk::Button,
    grip_slot: gtk::Box,
    pending_slot: gtk::Box,
}

/// What a change's card is told.
#[derive(Debug)]
pub(in crate::ui) enum ChangeCardInput {
    /// Draw what a change's card says (boxed: it is large beside the
    /// presses).
    Show(Box<ChangeWords>),
    /// People pressed a button (or Escape, or switched the mirror off).
    Pressed(ChangeAction),
    /// People pressed Enter: Apply, if it can be pressed.
    Enter,
}

/// What a change's card tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum ChangeCardOutput {
    /// People did this on the card.
    Action(ChangeAction),
}

impl Component for ChangeCardComponent {
    type Init = ChangeCardInit;
    type Input = ChangeCardInput;
    type Output = ChangeCardOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = ChangeCardWidgets;

    fn init_root() -> gtk::Box {
        gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-card", "stave-change-card"])
            .hexpand(true)
            .vexpand(true)
            .accessible_role(gtk::AccessibleRole::Group)
            .focusable(true)
            .visible(false)
            .build()
    }

    fn init(
        init: ChangeCardInit,
        root: gtk::Box,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(0)
            .css_classes(["stave-edit-body"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();
        let (buttons, cancel, add) = sections::footer_buttons();
        buttons.add_css_class("stave-edit-buttons");
        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-foot"])
            .build();
        foot.append(&buttons);
        root.append(&scrolled);
        root.append(&foot);
        // Shown, it takes the keyboard (Enter and Escape), unless the
        // keyboard is already in it.
        root.connect_map(|root| {
            if let Some(window) = root.root()
                && !window.focus().is_some_and(|focus| {
                    focus == *root.upcast_ref::<gtk::Widget>() || focus.is_ancestor(root)
                })
            {
                root.grab_focus();
            }
        });
        let input = sender.input_sender().clone();
        add.connect_clicked(move |_| input.emit(ChangeCardInput::Pressed(ChangeAction::Add)));
        let input = sender.input_sender().clone();
        cancel.connect_clicked(move |_| input.emit(ChangeCardInput::Pressed(ChangeAction::Cancel)));
        let keys = gtk::EventControllerKey::new();
        let input = sender.input_sender().clone();
        keys.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Escape => {
                input.emit(ChangeCardInput::Pressed(ChangeAction::Cancel));
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => {
                input.emit(ChangeCardInput::Enter);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });
        root.add_controller(keys);
        ComponentParts {
            model: ChangeCardComponent::default(),
            widgets: ChangeCardWidgets {
                body,
                add,
                grip_slot: init.grip_slot,
                pending_slot: init.pending_slot,
            },
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut ChangeCardWidgets,
        message: ChangeCardInput,
        sender: ComponentSender<Self>,
        root: &gtk::Box,
    ) {
        let action = match message {
            ChangeCardInput::Show(words) => {
                self.can_add = words.can_add;
                draw_change_card(widgets, root, &words, &sender);
                None
            }
            ChangeCardInput::Pressed(action) => Some(action),
            ChangeCardInput::Enter => self.can_add.then_some(ChangeAction::Add),
        };
        if let Some(action) = action {
            // Nobody listening is not a mistake.
            let _ = sender.output(ChangeCardOutput::Action(action));
        }
    }
}

/// Draws what a change's card says on its widgets.
fn draw_change_card(
    widgets: &ChangeCardWidgets,
    root: &gtk::Box,
    words: &ChangeWords,
    sender: &ComponentSender<ChangeCardComponent>,
) {
    let body = &widgets.body;
    while let Some(child) = body.first_child() {
        body.remove(&child);
    }
    root.update_property(&[gtk::accessible::Property::Label(&words.title)]);
    let kind = words
        .eyebrow
        .strip_prefix("CHANGE · ")
        .unwrap_or(&words.eyebrow)
        .to_lowercase();
    let note = text(&kind, &["stave-edit-origin"]);
    note.set_wrap(false);
    body.append(&sections::header(
        None,
        &sections::name_field(words.item.as_deref().unwrap_or(&words.title)),
        Some(note.upcast_ref()),
    ));
    let (section, change_body) = sections::section("Change", "", true);
    // The summary, unless the header already says it (a change of no one
    // item, such as a size a grip moves).
    if words.item.is_some() {
        change_body.append(&text(&words.title, &["stave-change-title"]));
    }
    for line in &words.lines {
        change_body.append(&text(line, &["stave-change-line"]));
    }
    if let Some(switch_words) = words.switch {
        let row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["stave-card", "stave-card-row"])
            .build();
        row.append(
            &gtk::Label::builder()
                .label(switch_words)
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        let switch = gtk::Switch::builder()
            .active(true)
            .valign(gtk::Align::Center)
            .tooltip_text("Turn it off to leave only the one side")
            .build();
        switch.update_property(&[gtk::accessible::Property::Label(switch_words)]);
        // Connected once it is on, so only people turning it off cancel.
        let input = sender.input_sender().clone();
        switch.connect_active_notify(move |switch| {
            if !switch.is_active() {
                input.emit(ChangeCardInput::Pressed(ChangeAction::Cancel));
            }
        });
        row.append(&switch);
        change_body.append(&row);
    }
    if words.waiting_on_dependants {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["stave-card", "stave-change-dependants"])
            .build();
        for name in &words.dependants {
            list.append(&text(&format!("• {name}"), &["stave-change-dependant"]));
        }
        change_body.append(&list);
        let include = gtk::Button::builder()
            .label(format!("Remove these {} too…", words.dependants.len()))
            .css_classes(["pill", "destructive-action"])
            .halign(gtk::Align::Start)
            .build();
        let input = sender.input_sender().clone();
        include.connect_clicked(move |_| {
            input.emit(ChangeCardInput::Pressed(ChangeAction::IncludeDependants));
        });
        change_body.append(&include);
    }
    if change_body.first_child().is_some() {
        body.append(&section);
    }
    body.append(&widgets.grip_slot);
    let (section, verdict_body) = sections::section("Verdict", "", true);
    verdict_body.append(&verdict_section(words, sender));
    body.append(&section);
    body.append(&widgets.pending_slot);
    let add = &widgets.add;
    add.set_sensitive(words.can_add);
    match words.commit {
        Commit::Remove => {
            add.remove_css_class("suggested-action");
            add.add_css_class("destructive-action");
        }
        Commit::Write => {
            add.remove_css_class("destructive-action");
            add.add_css_class("suggested-action");
        }
    }
    add.set_tooltip_text(Some(if words.can_add {
        "Write it into the design (Enter)"
    } else if words.waiting_on_dependants {
        "Say whether what uses it goes too first"
    } else {
        "It can be written once it fits"
    }));
    if root.is_mapped()
        && let Some(window) = root.root()
        && !window.focus().is_some_and(|focus| focus.is_ancestor(root))
    {
        root.grab_focus();
    }
}

/// The verdict section of a change's card: the status, then each problem
/// with its help and its fixes' buttons.
fn verdict_section(words: &ChangeWords, sender: &ComponentSender<ChangeCardComponent>) -> gtk::Box {
    let section = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["stave-edit-verdict"])
        .build();
    let status = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    section.append(&status);
    match &words.verdict {
        None => {
            status.append(&gtk::Image::from_icon_name("dialog-information-symbolic"));
            status.append(&text(
                words.needs.as_deref().unwrap_or("Not checked yet"),
                &["stave-description"],
            ));
        }
        Some(Verdict::Checking) => {
            status.append(&gtk::Spinner::builder().spinning(true).build());
            status.append(&text("Checking whether it fits…", &["stave-edit-status"]));
        }
        Some(Verdict::Fits(fits)) => {
            section.add_css_class("fits");
            status.append(&gtk::Image::from_icon_name("object-select-symbolic"));
            status.append(&text("It fits", &["stave-edit-status"]));
            for warning in fits.warnings() {
                section.append(&text(
                    &diagnostic_head(warning),
                    &["stave-edit-problem-head", "warning"],
                ));
            }
        }
        Some(Verdict::Refused(refusal)) => {
            section.add_css_class("refused");
            status.append(&gtk::Image::from_icon_name("dialog-error-symbolic"));
            let count = refusal.problems().iter().count();
            status.append(&text(
                &if count == 1 {
                    "It doesn't fit: 1 problem".to_owned()
                } else {
                    format!("It doesn't fit: {count} problems")
                },
                &["stave-edit-status"],
            ));
            for (problem_index, problem) in refusal.problems().iter().enumerate() {
                let diagnostic = problem.diagnostic();
                let head = text(
                    &diagnostic_head(diagnostic),
                    &["stave-edit-problem-head", "error"],
                );
                head.set_selectable(true);
                section.append(&head);
                if let Some(help) = &diagnostic.help {
                    section.append(&text(&format!("help: {help}"), &["stave-edit-help"]));
                }
                for (fix_index, fix) in problem.fixes().iter().enumerate() {
                    let button = gtk::Button::builder()
                        .label(fix.label())
                        .css_classes(["pill", "stave-edit-fix"])
                        .halign(gtk::Align::Start)
                        .tooltip_text("Add this change to what is tried")
                        .build();
                    let input = sender.input_sender().clone();
                    button.connect_clicked(move |_| {
                        input.emit(ChangeCardInput::Pressed(ChangeAction::Fix {
                            problem: problem_index,
                            fix: fix_index,
                        }));
                    });
                    section.append(&button);
                }
            }
        }
    }
    section
}

/// The card for a change of the design, behind the API the window still
/// uses: it owns the component (dropping it shuts the component down),
/// passes [`ChangeCard::show`] on as a message and the card's actions to
/// the callback.
pub(in crate::ui) struct ChangeCard {
    /// The card, in the Edit panel; its owner shows and hides it.
    pub(in crate::ui) root: gtk::Box,
    /// Where the grips put the size a drag moves (`grip_panel.rs`).
    pub(in crate::ui) grip_slot: gtk::Box,
    /// Where the window puts the "Pending" section.
    pub(in crate::ui) pending_slot: gtk::Box,
    bridge: Bridge<ChangeCardComponent>,
}

impl ChangeCard {
    /// An empty card, hidden.
    pub(in crate::ui) fn new() -> ChangeCard {
        let grip_slot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        let pending_slot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        let bridge = Bridge::<ChangeCardComponent>::launch(ChangeCardInit {
            grip_slot: grip_slot.clone(),
            pending_slot: pending_slot.clone(),
        });
        ChangeCard {
            root: bridge.widget().clone(),
            grip_slot,
            pending_slot,
            bridge,
        }
    }

    /// Calls `callback` with each thing the user does on the card.
    pub(in crate::ui) fn connect_action(&self, callback: impl Fn(ChangeAction) + 'static) {
        self.bridge.connect(move |output| match output {
            ChangeCardOutput::Action(action) => callback(action),
        });
    }

    /// Shows what a change's card says, when the main loop next runs.
    pub(in crate::ui) fn show(&self, words: &ChangeWords) {
        self.bridge
            .send(ChangeCardInput::Show(Box::new(words.clone())));
    }
}

fn text(words: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(classes.to_vec())
        .build()
}

/// What the removal's dialog says: its heading, the sentence before the
/// list, the list, and the button that removes them too.
pub(in crate::ui) fn dependants_words(
    item: &str,
    dependants: &[String],
) -> (String, String, Vec<String>, String) {
    (
        format!("Remove {item}?"),
        format!(
            "{} {item}, and can't stay without it:",
            if dependants.len() == 1 {
                "This uses"
            } else {
                "These use"
            }
        ),
        dependants.iter().map(|name| format!("• {name}")).collect(),
        format!(
            "Remove {item} and {}",
            if dependants.len() == 1 {
                "it".to_owned()
            } else {
                format!("all {}", dependants.len())
            }
        ),
    )
}

/// The dialog a removal opens while other things use the item: it names
/// each, and offers to remove them too or to leave everything as it is.
/// It floats in the middle of the 3D view over a dimmed backdrop and holds
/// the keyboard (Escape cancels), as a dialog does; it is part of the
/// window, so a screenshot of the window shows it.
///
/// A relm4 component: what it says is drawn from each
/// [`DependantsInput::Show`], and its buttons and Escape only send
/// [`DependantsInput::Answer`]. Its owner shows and hides the backdrop, and
/// alone decides whether an answer still counts, so the dialog keeps no
/// state of its own.
#[derive(Debug, Default)]
pub(in crate::ui) struct DependantsComponent;

/// The dialog's widgets.
#[derive(Debug)]
pub(in crate::ui) struct DependantsWidgets {
    heading: gtk::Label,
    before: gtk::Label,
    names: gtk::Box,
    cancel: gtk::Button,
    include: gtk::Button,
}

/// What the dialog is told.
#[derive(Debug)]
pub(in crate::ui) enum DependantsInput {
    /// Ask about removing `item`, which these use.
    Show {
        /// The item to remove.
        item: String,
        /// What uses it, by name, nearest first.
        dependants: Vec<String>,
    },
    /// People answered: `true` to remove the dependants too.
    Answer(bool),
}

/// What the dialog tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum DependantsOutput {
    /// Remove what uses the item too.
    Include,
    /// Leave everything as it is.
    Cancel,
}

impl Component for DependantsComponent {
    type Init = ();
    type Input = DependantsInput;
    type Output = DependantsOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = DependantsWidgets;

    fn init_root() -> gtk::Box {
        gtk::Box::builder()
            .css_classes(["stave-dependants-backdrop"])
            .hexpand(true)
            .vexpand(true)
            .visible(false)
            .build()
    }

    fn init(_: (), root: gtk::Box, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let heading = text("", &["title-2"]);
        let before = text("", &[]);
        let names = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .css_classes(["stave-card", "stave-change-dependants"])
            .build();
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .hexpand(true)
            .css_classes(["pill"])
            .build();
        let include = gtk::Button::builder()
            .hexpand(true)
            .css_classes(["pill", "destructive-action"])
            .build();
        let buttons = gtk::Box::builder()
            .spacing(12)
            .homogeneous(true)
            .margin_top(6)
            .build();
        buttons.append(&cancel);
        buttons.append(&include);
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .width_request(420)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["stave-card-panel", "stave-dependants-dialog"])
            .accessible_role(gtk::AccessibleRole::AlertDialog)
            .build();
        content.append(&heading);
        content.append(&before);
        content.append(&names);
        content.append(&text(
            "Nothing is removed until you press Apply in the Edit panel, after seeing it in red.",
            &["dim-label"],
        ));
        content.append(&buttons);
        root.append(&content);
        let input = sender.input_sender().clone();
        include.connect_clicked(move |_| input.emit(DependantsInput::Answer(true)));
        let input = sender.input_sender().clone();
        cancel.connect_clicked(move |_| input.emit(DependantsInput::Answer(false)));
        let keys = gtk::EventControllerKey::new();
        let input = sender.input_sender().clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            input.emit(DependantsInput::Answer(false));
            glib::Propagation::Stop
        });
        root.add_controller(keys);
        ComponentParts {
            model: DependantsComponent,
            widgets: DependantsWidgets {
                heading,
                before,
                names,
                cancel,
                include,
            },
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut DependantsWidgets,
        message: DependantsInput,
        sender: ComponentSender<Self>,
        _root: &gtk::Box,
    ) {
        match message {
            DependantsInput::Show { item, dependants } => {
                let (heading, before, list, include) = dependants_words(&item, &dependants);
                widgets.heading.set_label(&heading);
                widgets.before.set_label(&before);
                while let Some(child) = widgets.names.first_child() {
                    widgets.names.remove(&child);
                }
                for line in &list {
                    widgets
                        .names
                        .append(&text(line, &["stave-change-dependant"]));
                }
                widgets.include.set_label(&include);
                widgets.cancel.grab_focus();
            }
            DependantsInput::Answer(include) => {
                // Nobody listening is not a mistake.
                let _ = sender.output(if include {
                    DependantsOutput::Include
                } else {
                    DependantsOutput::Cancel
                });
            }
        }
    }
}

/// The removal's dialog, behind the API the window still uses: it owns the
/// component and passes the answers to the callback.
pub(in crate::ui) struct DependantsDialog {
    /// The backdrop, filling the 3D view, with the dialog in its middle;
    /// shown by [`DependantsDialog::show`], hidden by its owner.
    pub(in crate::ui) root: gtk::Box,
    bridge: Bridge<DependantsComponent>,
}

impl DependantsDialog {
    fn new() -> DependantsDialog {
        let bridge = Bridge::<DependantsComponent>::launch(());
        DependantsDialog {
            root: bridge.widget().clone(),
            bridge,
        }
    }

    /// Shows the dialog at once, asking about `item`; what it says is
    /// drawn when the main loop next runs.
    fn show(&self, item: &str, dependants: &[String]) {
        self.root.set_visible(true);
        self.bridge.send(DependantsInput::Show {
            item: item.to_owned(),
            dependants: dependants.to_vec(),
        });
    }

    /// Calls `callback` with each answer: `true` to remove the dependants
    /// too.
    fn connect_answered(&self, callback: impl Fn(bool) + 'static) {
        self.bridge.connect(move |output| match output {
            DependantsOutput::Include => callback(true),
            DependantsOutput::Cancel => callback(false),
        });
    }
}

impl Window {
    /// Connects the change's card (in the Edit panel) and puts the
    /// removal's dialog over the 3D view.
    pub(in crate::ui) fn connect_changes(&self) {
        install_style();
        let card = &self.editing.changes.card;
        let dialog = &self.editing.changes.dialog;
        self.widgets.design.add_overlay(&dialog.root);
        let input = self.input();
        dialog.connect_answered(move |include| {
            input.emit(EditingInput::DependantsAnswered(include).into());
        });
        let input = self.input();
        card.connect_action(move |action| input.emit(EditingInput::ChangeAction(action).into()));
    }

    /// Draws the change in progress on its card, or hides the card.
    /// Returns whether a change is in progress.
    pub(in crate::ui) fn draw_change(&mut self, checking: bool) -> bool {
        let words = {
            let session = &self.session;
            session.change().map(|change| {
                (
                    change_words(change, checking),
                    change_item(change.request()),
                )
            })
        };
        let card = &self.editing.changes.card;
        let Some((words, item)) = words else {
            card.root.set_visible(false);
            self.editing.changes.asked.replace(None);
            self.close_change_dialog();
            return false;
        };
        card.root.set_visible(true);
        card.show(&words);
        if words.waiting_on_dependants {
            self.ask_about_dependants(item.unwrap_or_default(), &words.dependants);
        } else {
            self.close_change_dialog();
        }
        true
    }

    /// Asks, once a removal, whether what uses the item goes too.
    fn ask_about_dependants(&mut self, item: String, dependants: &[String]) {
        if self.editing.changes.asked.borrow().as_deref() == Some(item.as_str()) {
            return;
        }
        self.editing.changes.asked.replace(Some(item.clone()));
        self.editing.changes.dialog.show(&item, dependants);
    }

    /// The removal's dialog was answered: remove what uses the item too,
    /// or leave everything as it is.
    pub(in crate::ui) fn answer_dependants(&mut self, include: bool) {
        let dialog = &self.editing.changes.dialog;
        if !dialog.root.is_visible() {
            return;
        }
        dialog.root.set_visible(false);
        self.send_edit_later(if include {
            EditCommand::IncludeDependants
        } else {
            EditCommand::Cancel
        });
    }

    /// Closes the removal's dialog, if it is open (the change ended some
    /// other way).
    pub(in crate::ui) fn close_change_dialog(&self) {
        self.editing.changes.dialog.root.set_visible(false);
    }

    /// Draws a frame for a removal: the design as it is, with what would
    /// go see-through and red, so nothing is removed without being seen.
    /// `None` for anything else.
    pub(in crate::ui) fn render_removal(
        &mut self,
        backdrop: crate::render::Backdrop,
    ) -> Option<anyhow::Result<crate::render::Ticket>> {
        let session = &self.session;
        let change = session.change()?;
        if !matches!(change.request(), ChangeRequest::Remove { .. }) {
            return None;
        }
        let mut edit_state = self.editing.state.borrow_mut();
        let ghost = edit_state.ghost.as_mut()?;
        let removed: Vec<stave_scene::NodeId> = ghost.ghosts.removed().to_vec();
        if removed.is_empty() {
            return None;
        }
        let shown = session.shown()?.clone();
        let (_, graph) = shown.drawable()?;
        let state = &mut self.state;
        if state.focus.is_some() {
            return None;
        }
        let orbit = state.orbit?;
        let (width, height) = state.size;
        let super::super::State {
            renderer,
            uploaded,
            render_queued,
            posed,
            ..
        } = &mut *state;
        let renderer = renderer.as_mut().ok()?;
        let posed = posed.as_ref()?;
        *render_queued = false;
        if uploaded
            .as_ref()
            .is_none_or(|held| !Rc::ptr_eq(held, &shown))
        {
            renderer.upload(graph);
            *uploaded = Some(shown.clone());
        }
        // The candidate's scene is no longer in the renderer.
        ghost.uploaded = false;
        let count = graph.node_ids().count();
        let mut looks = vec![stave_scene::Look::Normal; count];
        let mut tints = vec![[0.0; 4]; count];
        let [r, g, b] = stave_scene::GhostVerdict::Refused.tint();
        for node in removed {
            if let Some(look) = looks.get_mut(node.index()) {
                *look = stave_scene::Look::XRay;
            }
            if let Some(tint) = tints.get_mut(node.index()) {
                *tint = [r, g, b, 0.7];
            }
        }
        Some(renderer.submit(&crate::render::Frame {
            scene: graph,
            posed,
            orbit,
            selection: &[],
            highlight: None,
            section: None,
            looks: Some(&looks),
            overlay: None,
            tint_cut_faces: true,
            lit: None,
            tints: Some(&tints),
            backdrop,
            width,
            height,
        }))
    }
}

/// The look of the changes' controls (the handles over the 3D view, the
/// pending lines in the editor, the item's tools), in libadwaita's named
/// colours so light and dark both follow the style manager.
const STYLE: &str = r#"
.stave-repeat-handle { padding: 3px 10px; border-radius: 999px; font-weight: 700; font-size: 13px;
  background: @accent_bg_color; color: @accent_fg_color; box-shadow: 0 1px 4px alpha(black, 0.35); }
.stave-repeat-handle.boundary { background: @window_bg_color; color: @accent_color;
  border: 2px solid @accent_bg_color; }
.stave-item-tools > .stave-section { margin-top: 4px; }
.stave-card > .stave-card-row.stave-item-buttons { padding-top: 2px; padding-bottom: 2px; }
.stave-change-line { font-size: 12px; color: alpha(currentColor, 0.8); }
.stave-change-title { font-size: 12px; font-weight: 700; }
.stave-change-dependants { padding: 8px 12px; }
.stave-change-dependant { font-family: monospace; font-size: 13px; }
.stave-dependants-backdrop { background: alpha(black, 0.32); }
.stave-dependants-dialog { padding: 24px; }
.stave-hold-from-space { margin: 0 12px 10px 12px; }
.stave-repeat-on-card { padding: 4px 16px 0 16px; }
.stave-pending-mark { font-weight: 700; }
.stave-pending-old-value { font-weight: 700; text-decoration-line: line-through; }
.stave-pending-new-value { font-weight: 700; }
"#;

/// Puts the changes' stylesheet on the display, once.
fn install_style() {
    thread_local! {
        static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    let Some(display) = gdk::Display::default() else {
        return;
    };
    if INSTALLED.with(std::cell::Cell::get) {
        return;
    }
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLE);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
    );
    INSTALLED.with(|installed| installed.set(true));
}

/// The change card and what only it needs.
pub(in crate::ui) struct Changes {
    /// The card.
    pub(in crate::ui) card: ChangeCard,
    /// The item whose removal the dialog last asked about, so it asks once.
    asked: RefCell<Option<String>>,
    /// The removal's dialog.
    pub(in crate::ui) dialog: DependantsDialog,
}

impl Changes {
    /// The card, hidden.
    pub(in crate::ui) fn new() -> Changes {
        Changes {
            card: ChangeCard::new(),
            asked: RefCell::new(None),
            dialog: DependantsDialog::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_action_is_the_sessions_command() {
        assert_eq!(ChangeAction::Add.command(), EditCommand::Add);
        assert_eq!(ChangeAction::Cancel.command(), EditCommand::Cancel);
        assert_eq!(
            ChangeAction::IncludeDependants.command(),
            EditCommand::IncludeDependants
        );
        assert_eq!(
            ChangeAction::Fix { problem: 1, fix: 0 }.command(),
            EditCommand::ApplyFix { problem: 1, fix: 0 }
        );
    }

    #[test]
    fn the_dialog_names_everything_that_goes() {
        let (heading, before, list, include) =
            dependants_words("back", &["drawer_bay".to_owned(), "drawer".to_owned()]);
        assert_eq!(heading, "Remove back?");
        assert_eq!(before, "These use back, and can't stay without it:");
        assert_eq!(list, vec!["• drawer_bay", "• drawer"]);
        assert_eq!(include, "Remove back and all 2");
        let (_, before, _, include) = dependants_words("back", &["drawer".to_owned()]);
        assert_eq!(before, "This uses back, and can't stay without it:");
        assert_eq!(include, "Remove back and it");
    }

    use std::cell::RefCell;

    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// What a rename's card says, with Apply ready.
    fn rename_words() -> ChangeWords {
        ChangeWords {
            eyebrow: "CHANGE · RENAME".to_owned(),
            title: "Rename wall to back_wall".to_owned(),
            item: Some("wall".to_owned()),
            lines: vec!["wall becomes back_wall everywhere it is used".to_owned()],
            dependants: Vec::new(),
            waiting_on_dependants: false,
            verdict: None,
            needs: None,
            commit: Commit::Write,
            can_add: true,
            switch: None,
        }
    }

    /// The buttons under `widget` that have this CSS class, in order.
    fn buttons_with(widget: &gtk::Widget, class: &str) -> Vec<gtk::Button> {
        let mut found = Vec::new();
        let mut stack = vec![widget.clone()];
        while let Some(widget) = stack.pop() {
            if widget.has_css_class(class)
                && let Ok(button) = widget.clone().downcast::<gtk::Button>()
            {
                found.push(button);
            }
            let mut child = widget.last_child();
            while let Some(next) = child {
                child = next.prev_sibling();
                stack.push(next);
            }
        }
        found
    }

    /// The card's Cancel and Apply.
    fn footer(card: &ChangeCard) -> (gtk::Button, gtk::Button) {
        match buttons_with(card.root.upcast_ref(), "stave-footer-button").as_slice() {
            [cancel, apply] => (cancel.clone(), apply.clone()),
            other => panic!("the card has Cancel and Apply, not {} buttons", other.len()),
        }
    }

    #[test]
    fn a_dropped_change_card_is_freed() {
        on_gtk_thread(change_card_is_freed);
    }

    /// Dropping the card drops its controller: the widget, the model and
    /// the callback are freed once the main loop runs.
    fn change_card_is_freed() {
        let card = ChangeCard::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        card.connect_action(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        card.show(&rename_words());
        settle();
        let root = card.root.downgrade();
        let input = card.bridge.sender().clone();
        drop(card);
        settle();
        assert!(root.upgrade().is_none(), "the card is freed");
        assert!(
            input.send(ChangeCardInput::Enter).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn showing_a_change_card_reports_nothing() {
        on_gtk_thread(showing_a_card_reports_nothing);
    }

    fn showing_a_card_reports_nothing() {
        let card = ChangeCard::new();
        let heard: Rc<RefCell<Vec<ChangeAction>>> = Rc::default();
        let log = Rc::clone(&heard);
        card.connect_action(move |action| log.borrow_mut().push(action));
        let mut words = rename_words();
        words.switch = Some("And the other side");
        card.show(&words);
        card.show(&rename_words());
        settle();
        let (_, apply) = footer(&card);
        assert!(apply.is_sensitive(), "Apply follows the words");
        assert!(heard.borrow().is_empty(), "drawing it does nothing");
        // Enter presses Apply only while it can be pressed.
        let mut waiting = rename_words();
        waiting.can_add = false;
        card.show(&waiting);
        card.bridge.send(ChangeCardInput::Enter);
        settle();
        assert!(!apply.is_sensitive());
        assert!(heard.borrow().is_empty(), "Enter did nothing");
    }

    #[test]
    fn a_press_made_while_a_press_is_handled_is_queued() {
        on_gtk_thread(presses_are_queued);
    }

    /// A press only queues a message; the action reaches the callback
    /// after the card's update has returned. The callback redraws the card
    /// and presses Cancel, as a window handling Apply might: both wait
    /// their turn instead of running under it.
    fn presses_are_queued() {
        let card = Rc::new(ChangeCard::new());
        card.show(&rename_words());
        settle();
        let heard: Rc<RefCell<Vec<ChangeAction>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&card);
        card.connect_action(move |action| {
            let Some(card) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            log.borrow_mut().push(action);
            if first {
                card.show(&rename_words());
                footer(&card).0.emit_clicked();
                assert_eq!(log.borrow().len(), 1, "nothing ran under the callback");
            }
        });
        footer(&card).1.emit_clicked();
        assert!(heard.borrow().is_empty(), "the press is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![ChangeAction::Add, ChangeAction::Cancel],
            "each press after its update, the inner one after the first"
        );
    }

    #[test]
    fn a_dropped_dependants_dialog_is_freed() {
        on_gtk_thread(dependants_dialog_is_freed);
    }

    fn dependants_dialog_is_freed() {
        let dialog = DependantsDialog::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        dialog.connect_answered(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        dialog.show("back", &["drawer".to_owned()]);
        assert!(dialog.root.is_visible(), "shown at once");
        settle();
        let root = dialog.root.downgrade();
        let input = dialog.bridge.sender().clone();
        drop(dialog);
        settle();
        assert!(root.upgrade().is_none(), "the dialog is freed");
        assert!(
            input.send(DependantsInput::Answer(false)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn an_answer_given_while_an_answer_is_handled_is_queued() {
        on_gtk_thread(answers_are_queued);
    }

    /// The old dialog's buttons called the window from inside their own
    /// signal. Now an answer is queued, and an answer given from inside
    /// the callback (a second click) waits its turn.
    fn answers_are_queued() {
        let dialog = Rc::new(DependantsDialog::new());
        dialog.show("back", &["drawer_bay".to_owned(), "drawer".to_owned()]);
        settle();
        let heard: Rc<RefCell<Vec<bool>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&dialog);
        dialog.connect_answered(move |include| {
            let Some(dialog) = weak.upgrade() else {
                return;
            };
            let first = log.borrow().is_empty();
            log.borrow_mut().push(include);
            if first {
                if let Some(cancel) = buttons_with(dialog.root.upcast_ref(), "pill").first() {
                    cancel.emit_clicked();
                }
                assert_eq!(log.borrow().len(), 1, "nothing ran under the callback");
            }
        });
        let buttons = buttons_with(dialog.root.upcast_ref(), "pill");
        let Some(include) = buttons.get(1) else {
            panic!("the dialog has Cancel and the button that removes them too");
        };
        assert_eq!(include.label().as_deref(), Some("Remove back and all 2"));
        include.emit_clicked();
        assert!(heard.borrow().is_empty(), "the answer is only queued");
        settle();
        assert_eq!(*heard.borrow(), vec![true, false]);
    }
}
