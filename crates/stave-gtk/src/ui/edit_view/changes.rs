//! The card for a change of the design (`docs/editing.md`, "Changes of
//! the design"): a repeat, the shares grip, a mirror, a held distance, a
//! removal, a rename. It draws what [`Session::change`] holds after each
//! [`EditEvent`] (what the change does, what it found, the verdict with the
//! checker's problems and fixes) and sends the session's commands for its
//! buttons. A removal that something still uses asks first, in a dialog
//! that names each thing that would go with it.
//!
//! ```text
//! ┌ CHANGE · REMOVE ───────────────────────┐
//! │ Remove back                            │
//! │  Also removes: drawer_bay, drawer      │
//! │ ✓ It fits                              │
//! │                     [Cancel] [Remove]  │
//! └────────────────────────────────────────┘
//! ```

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use stave_app::change::{Change, ChangeDetail, ChangeRequest};
use stave_app::edit::Verdict;
use stave_app::editing::{EditCommand, Progress};
use stave_app::remove::Dependants;

use super::super::Window;
use super::super::edit_card::diagnostic_head;
use super::card_verdict;

type ActionCallback = Rc<dyn Fn(ChangeAction)>;

/// What the user did on a change's card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum ChangeAction {
    /// Pressed the card's commit button (Add, Remove, Rename).
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

/// What a change's card says, worked out from the change alone (so it is
/// tested without a window).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct ChangeWords {
    /// "CHANGE · REPEAT".
    pub(in crate::ui) eyebrow: String,
    /// The change's summary: "Make 3 of drawer".
    pub(in crate::ui) title: String,
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
    /// The commit button's label.
    pub(in crate::ui) add_label: &'static str,
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
    let mut add_label = "Add";
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
            add_label = "Hold";
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
            add_label = "Remove";
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
            add_label = "Rename";
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
        ChangeDetail::Grip(detail) => {
            add_label = "Set";
            for change in &detail.changes {
                lines.push(format!("{} changes", change.path));
            }
            if let Some(stopped) = &detail.stopped {
                lines.push(stopped.clone());
            }
        }
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
        lines,
        dependants,
        waiting_on_dependants,
        verdict,
        needs,
        add_label,
        can_add,
        switch: matches!(request, ChangeRequest::Mirror { .. }).then_some("And the other side"),
    }
}

/// The card for a change of the design.
pub(in crate::ui) struct ChangeCard {
    /// The card, in the Edit panel.
    pub(in crate::ui) root: gtk::Box,
    body: gtk::Box,
    add: gtk::Button,
    on_action: Rc<RefCell<Option<ActionCallback>>>,
    /// Each fix's button, with its problem and fix index (for tests).
    fixes: RefCell<Vec<((usize, usize), gtk::Button)>>,
}

impl ChangeCard {
    /// An empty card, hidden.
    pub(in crate::ui) fn new() -> ChangeCard {
        let on_action: Rc<RefCell<Option<ActionCallback>>> = Rc::new(RefCell::new(None));
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["stave-edit-body"])
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&body)
            .build();
        let cancel = gtk::Button::builder()
            .label("Cancel")
            .tooltip_text("Leave the design as it is (Escape)")
            .build();
        let add = gtk::Button::builder()
            .label("Add")
            .css_classes(["suggested-action"])
            .build();
        let buttons = gtk::Box::builder()
            .spacing(8)
            .halign(gtk::Align::End)
            .css_classes(["stave-edit-buttons"])
            .build();
        buttons.append(&cancel);
        buttons.append(&add);
        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-foot"])
            .build();
        foot.append(&buttons);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-edit-card", "stave-change-card"])
            .hexpand(true)
            .vexpand(true)
            .accessible_role(gtk::AccessibleRole::Group)
            .focusable(true)
            .visible(false)
            .build();
        root.append(&scrolled);
        root.append(&foot);
        root.connect_map(|root| {
            if let Some(window) = root.root()
                && !window.focus().is_some_and(|focus| {
                    focus == *root.upcast_ref::<gtk::Widget>() || focus.is_ancestor(root)
                })
            {
                root.grab_focus();
            }
        });
        let send = {
            let on_action = on_action.clone();
            move |action: ChangeAction| {
                let callback = on_action.borrow().clone();
                if let Some(callback) = callback {
                    callback(action);
                }
            }
        };
        {
            let send = send.clone();
            add.connect_clicked(move |_| send(ChangeAction::Add));
        }
        {
            let send = send.clone();
            cancel.connect_clicked(move |_| send(ChangeAction::Cancel));
        }
        let keys = gtk::EventControllerKey::new();
        {
            let add = add.clone();
            keys.connect_key_pressed(move |_, key, _, _| match key {
                gdk::Key::Escape => {
                    send(ChangeAction::Cancel);
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => {
                    if add.is_sensitive() {
                        send(ChangeAction::Add);
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            });
        }
        root.add_controller(keys);
        ChangeCard {
            root,
            body,
            add,
            on_action,
            fixes: RefCell::new(Vec::new()),
        }
    }

    /// Calls `callback` with each thing the user does on the card.
    pub(in crate::ui) fn connect_action(&self, callback: impl Fn(ChangeAction) + 'static) {
        self.on_action.replace(Some(Rc::new(callback)));
    }

    /// Shows what a change's card says.
    pub(in crate::ui) fn show(&self, words: &ChangeWords) {
        while let Some(child) = self.body.first_child() {
            self.body.remove(&child);
        }
        self.root
            .update_property(&[gtk::accessible::Property::Label(&words.title)]);
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head.append(&text(&words.eyebrow, &["stave-edit-caption"]));
        head.append(&text(&words.title, &["stave-edit-title"]));
        self.body.append(&head);
        if !words.lines.is_empty() {
            let lines = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(4)
                .css_classes(["stave-change-lines"])
                .build();
            for line in &words.lines {
                lines.append(&text(line, &["stave-change-line"]));
            }
            self.body.append(&lines);
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
            let card = self.on_action.clone();
            switch.connect_active_notify(move |switch| {
                if switch.is_active() {
                    return;
                }
                let callback = card.borrow().clone();
                if let Some(callback) = callback {
                    callback(ChangeAction::Cancel);
                }
            });
            row.append(&switch);
            self.body.append(&row);
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
            self.body.append(&list);
            let include = gtk::Button::builder()
                .label(format!("Remove these {} too…", words.dependants.len()))
                .css_classes(["pill", "destructive-action"])
                .halign(gtk::Align::Start)
                .build();
            let card = self.on_action.clone();
            include.connect_clicked(move |_| {
                let callback = card.borrow().clone();
                if let Some(callback) = callback {
                    callback(ChangeAction::IncludeDependants);
                }
            });
            self.body.append(&include);
        }
        let mut fixes = Vec::new();
        self.body.append(&self.verdict_section(words, &mut fixes));
        self.fixes.replace(fixes);
        self.add.set_label(words.add_label);
        self.add.set_sensitive(words.can_add);
        if words.add_label == "Remove" {
            self.add.remove_css_class("suggested-action");
            self.add.add_css_class("destructive-action");
        } else {
            self.add.remove_css_class("destructive-action");
            self.add.add_css_class("suggested-action");
        }
        self.add.set_tooltip_text(Some(if words.can_add {
            "Write it into the design (Enter)"
        } else if words.waiting_on_dependants {
            "Say whether what uses it goes too first"
        } else {
            "It can be written once it fits"
        }));
        if self.root.is_mapped()
            && let Some(window) = self.root.root()
            && !window
                .focus()
                .is_some_and(|focus| focus.is_ancestor(&self.root))
        {
            self.root.grab_focus();
        }
    }

    fn verdict_section(
        &self,
        words: &ChangeWords,
        fixes: &mut Vec<((usize, usize), gtk::Button)>,
    ) -> gtk::Box {
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
                        let card = self.on_action.clone();
                        button.connect_clicked(move |_| {
                            let callback = card.borrow().clone();
                            if let Some(callback) = callback {
                                callback(ChangeAction::Fix {
                                    problem: problem_index,
                                    fix: fix_index,
                                });
                            }
                        });
                        section.append(&button);
                        fixes.push(((problem_index, fix_index), button));
                    }
                }
            }
        }
        section
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
pub(in crate::ui) struct DependantsDialog {
    /// The backdrop, filling the 3D view, with the dialog in its middle.
    pub(in crate::ui) root: gtk::Box,
    heading: gtk::Label,
    before: gtk::Label,
    names: gtk::Box,
    cancel: gtk::Button,
    include: gtk::Button,
}

impl DependantsDialog {
    fn new() -> DependantsDialog {
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
            "Nothing is removed until you press Remove on the card, after seeing it in red.",
            &["dim-label"],
        ));
        content.append(&buttons);
        let root = gtk::Box::builder()
            .css_classes(["stave-dependants-backdrop"])
            .hexpand(true)
            .vexpand(true)
            .visible(false)
            .build();
        root.append(&content);
        DependantsDialog {
            root,
            heading,
            before,
            names,
            cancel,
            include,
        }
    }

    fn show(&self, item: &str, dependants: &[String]) {
        let (heading, before, list, include) = dependants_words(item, dependants);
        self.heading.set_label(&heading);
        self.before.set_label(&before);
        while let Some(child) = self.names.first_child() {
            self.names.remove(&child);
        }
        for line in &list {
            self.names.append(&text(line, &["stave-change-dependant"]));
        }
        self.include.set_label(&include);
        self.root.set_visible(true);
        self.cancel.grab_focus();
    }
}

impl Window {
    /// Connects the change's card (in the Edit panel) and puts the
    /// removal's dialog over the 3D view.
    pub(in crate::ui) fn connect_changes(self: &Rc<Self>) {
        install_style();
        let card = &self.editing.changes.card;
        let dialog = &self.editing.changes.dialog;
        self.widgets.design.add_overlay(&dialog.root);
        let window = Rc::downgrade(self);
        dialog.include.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.answer_dependants(true);
            }
        });
        let window = Rc::downgrade(self);
        dialog.cancel.connect_clicked(move |_| {
            if let Some(window) = window.upgrade() {
                window.answer_dependants(false);
            }
        });
        let keys = gtk::EventControllerKey::new();
        let window = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if let Some(window) = window.upgrade() {
                window.answer_dependants(false);
            }
            glib::Propagation::Stop
        });
        dialog.root.add_controller(keys);
        let window = Rc::downgrade(self);
        card.connect_action(move |action| {
            if let Some(window) = window.upgrade() {
                window.send_edit_later(action.command());
            }
        });
    }

    /// Draws the change in progress on its card, or hides the card.
    /// Returns whether a change is in progress.
    pub(in crate::ui) fn draw_change(self: &Rc<Self>, checking: bool) -> bool {
        let words = {
            let session = self.session.borrow();
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
    fn ask_about_dependants(self: &Rc<Self>, item: String, dependants: &[String]) {
        if self.editing.changes.asked.borrow().as_deref() == Some(item.as_str()) {
            return;
        }
        self.editing.changes.asked.replace(Some(item.clone()));
        self.editing.changes.dialog.show(&item, dependants);
    }

    /// The removal's dialog was answered: remove what uses the item too,
    /// or leave everything as it is.
    pub(in crate::ui) fn answer_dependants(self: &Rc<Self>, include: bool) {
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
        &self,
        backdrop: crate::render::Backdrop,
    ) -> Option<anyhow::Result<crate::render::Ticket>> {
        let session = self.session.borrow();
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
        let mut state = self.state.borrow_mut();
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
.stave-change-line { font-size: 13px; }
.stave-change-dependants { padding: 8px 12px; }
.stave-change-dependant { font-family: monospace; font-size: 13px; }
.stave-dependants-backdrop { background: alpha(black, 0.32); }
.stave-dependants-dialog { padding: 24px; }
.stave-hold-from-space { margin: 0 12px 10px 12px; }
.stave-repeat-on-card { padding: 4px 16px 0 16px; }
.stave-pending { padding: 8px 12px; margin: 6px 10px 2px 10px; border-radius: 8px;
  background: alpha(@accent_bg_color, 0.08); border: 1px dashed alpha(@accent_bg_color, 0.6); }
.stave-pending-head { font-size: 11px; font-weight: 700; letter-spacing: 0.04em; color: @accent_color; }
.stave-pending-lines { font-family: monospace; font-size: 12.5px; }
.stave-pending-mark { font-weight: 700; min-width: 1em; }
.stave-pending-added { color: @success_color; }
.stave-pending-removed { color: @error_color; }
.stave-pending-removed .stave-pending-text { text-decoration-line: line-through; }
.stave-pending-old-value { color: @error_color; text-decoration-line: line-through; }
.stave-pending-new-value { color: @success_color; font-weight: 700; }
.stave-pending-more { opacity: 0.7; }
.stave-pending-place { opacity: 0.6; font-size: 11px; margin-top: 2px; }
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
}
