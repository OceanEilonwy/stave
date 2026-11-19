//! The menu shown for a selection (`docs/editing.md`, "Signature
//! matching"): what can go in this space, on this face or between these
//! parts. It draws the candidates it is given (`stave_app::edit`) and
//! reports which one the user chose; it decides nothing about them.
//!
//! ```text
//! ┌ In this space ──────────────────────────────┐
//! │ CASEWORK                                     │
//! │  Drawer                                      │
//! │  A drawer on side-mount slides: an applied…  │
//! │  FramePanelDoor                   ◌ checking │
//! │  …                                           │
//! └──────────────────────────────────────────────┘
//! ```
//!
//! The heading comes from the selection's type ([`Heading`]); entries are
//! grouped by module, the project's own first, then imported modules by
//! name, then the standard ones; within a group, entries that fit come
//! first, then those still being checked, then the greyed ones, each in
//! name order. A greyed entry says the checker's reason and can't be
//! chosen; an entry not checked yet says so.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::edit::{Applicability, Candidate, Declaration, Heading, MenuGroup};
use stave_lang::Diagnostic;
use stave_lang::ast::Name;
use stave_model::nonempty::NonEmpty;

use super::edit_card::install_style;

type ChosenCallback = Rc<dyn Fn(Declaration)>;

/// Whether a candidate's applicability check has finished. Until the check
/// is done the caller says so here, and the entry shows that it is being
/// checked, as it does for [`Applicability::NotChecked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checked {
    /// The candidate's `applicability` is the checker's answer.
    Done,
    /// Its check hasn't finished: the entry can't be chosen yet.
    Pending,
}

/// One entry to show: a candidate and whether its check has finished.
#[derive(Debug, Clone, Copy)]
pub struct MenuItem<'a> {
    /// The candidate.
    pub candidate: &'a Candidate,
    /// Whether `candidate.applicability` is final.
    pub checked: Checked,
}

/// How an entry is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EntryState<'a> {
    /// It applies: it can be chosen.
    Fits,
    /// It can be chosen, and the card then asks for these (place params
    /// to point at, settings to choose) before it is tried.
    NeedsChoice(&'a NonEmpty<Name>),
    /// Still being checked.
    Checking,
    /// It doesn't apply here, with the checker's reasons.
    Greyed(&'a NonEmpty<Diagnostic>),
}

impl<'a> EntryState<'a> {
    fn of(item: &MenuItem<'a>) -> Self {
        match (item.checked, &item.candidate.applicability) {
            (Checked::Pending, _) | (Checked::Done, Applicability::NotChecked) => {
                EntryState::Checking
            }
            // Something is still to choose: not a refusal. The entry is
            // listed as usual and the card asks for it once it is chosen.
            (Checked::Done, Applicability::Applies) => EntryState::Fits,
            (Checked::Done, Applicability::NeedsChoice(names)) => EntryState::NeedsChoice(names),
            (Checked::Done, Applicability::Greyed(reasons)) => EntryState::Greyed(reasons),
        }
    }

    /// Where it sorts within its group: fits, checking, greyed.
    fn rank(self) -> u8 {
        match self {
            EntryState::Fits | EntryState::NeedsChoice(_) => 0,
            EntryState::Checking => 1,
            EntryState::Greyed(_) => 2,
        }
    }
}

/// Where a group sorts: the project, imported modules by name, then the
/// standard modules by name.
fn group_order(group: &MenuGroup) -> (u8, &str) {
    match group {
        MenuGroup::Project => (0, ""),
        MenuGroup::Module(name) => (1, name.as_str()),
        MenuGroup::Standard(name) => (2, name.as_str()),
    }
}

/// The words over a group: "In this project", "casework", "Standard ·
/// joinery".
pub fn group_title(group: &MenuGroup) -> String {
    stave_app::choices::group_title(group)
}

/// The entries grouped and ordered as the menu lists them.
fn sections<'i, 'a>(items: &'i [MenuItem<'a>]) -> Vec<(&'a MenuGroup, Vec<&'i MenuItem<'a>>)> {
    let mut ordered: Vec<&MenuItem<'a>> = items.iter().collect();
    ordered.sort_by(|left, right| {
        group_order(&left.candidate.entry.group)
            .cmp(&group_order(&right.candidate.entry.group))
            .then_with(|| {
                EntryState::of(left)
                    .rank()
                    .cmp(&EntryState::of(right).rank())
            })
            .then_with(|| {
                left.candidate
                    .entry
                    .label
                    .as_str()
                    .cmp(right.candidate.entry.label.as_str())
            })
    });
    let mut sections: Vec<(&'a MenuGroup, Vec<&MenuItem<'a>>)> = Vec::new();
    for item in ordered {
        let group = &item.candidate.entry.group;
        match sections.last_mut() {
            Some((last, entries)) if *last == group => entries.push(item),
            _ => sections.push((group, vec![item])),
        }
    }
    sections
}

/// The line under a greyed entry: the checker's messages, one a line.
fn reason_text(reasons: &NonEmpty<Diagnostic>) -> String {
    reasons
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The line under an entry that needs a choice: "Then point at or
/// choose: hinge side, mount".
fn needs_choice_text(names: &NonEmpty<Name>) -> String {
    let names: Vec<String> = names
        .iter()
        .map(|name| name.as_str().replace('_', " "))
        .collect();
    format!("Then point at or choose: {}", names.join(", "))
}

/// The menu for a selection.
pub struct EditMenu {
    /// The panel, ready to float over the 3D view.
    pub root: gtk::Box,
    heading: gtk::Label,
    list: gtk::Box,
    /// The entries shown, in order, with their buttons.
    rows: RefCell<Vec<(Declaration, gtk::Button)>>,
    on_chosen: Rc<RefCell<Option<ChosenCallback>>>,
}

impl EditMenu {
    /// An empty menu.
    pub fn new() -> EditMenu {
        if let Some(display) = gtk::gdk::Display::default() {
            install_style(&display);
        }
        let heading = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["stave-inspector-title"])
            .build();
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(14)
            .build();
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .css_classes(["stave-inspector"])
            .build();
        inner.append(&heading);
        inner.append(&list);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(560)
            .child(&inner)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-card-panel", "stave-edit-menu"])
            .width_request(320)
            .accessible_role(gtk::AccessibleRole::Group)
            .overflow(gtk::Overflow::Hidden)
            .build();
        root.append(&scrolled);
        EditMenu {
            root,
            heading,
            list,
            rows: RefCell::new(Vec::new()),
            on_chosen: Rc::new(RefCell::new(None)),
        }
    }

    /// Calls `callback` with the declaration of the entry the user chose.
    /// Greyed entries and entries still being checked never call it.
    pub fn connect_chosen(&self, callback: impl Fn(Declaration) + 'static) {
        self.on_chosen.replace(Some(Rc::new(callback)));
    }

    /// Shows the menu for a selection of this kind with these entries.
    pub fn show(&self, heading: Heading, items: &[MenuItem<'_>]) {
        self.heading.set_label(heading.text());
        self.root
            .update_property(&[gtk::accessible::Property::Label(heading.text())]);
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut rows = Vec::new();
        if items.is_empty() {
            self.list.append(
                &gtk::Label::builder()
                    .label("Nothing in this project or its imports goes here.")
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["stave-description"])
                    .build(),
            );
        }
        for (group, entries) in sections(items) {
            let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
            section.append(
                &gtk::Label::builder()
                    .label(group_title(group).to_uppercase())
                    .xalign(0.0)
                    .css_classes(["stave-section"])
                    .build(),
            );
            let card = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .css_classes(["stave-card"])
                .overflow(gtk::Overflow::Hidden)
                .build();
            for item in entries {
                let button = self.entry_button(item);
                card.append(&button);
                rows.push((item.candidate.declaration, button));
            }
            section.append(&card);
            self.list.append(&section);
        }
        self.rows.replace(rows);
    }

    /// One entry: its label, its hint and, when it can't be chosen, why.
    fn entry_button(&self, item: &MenuItem<'_>) -> gtk::Button {
        let entry = &item.candidate.entry;
        let state = EntryState::of(item);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.append(
            &gtk::Label::builder()
                .label(entry.label.as_str())
                .xalign(0.0)
                .hexpand(true)
                .css_classes(["stave-edit-entry-label"])
                .build(),
        );
        if state == EntryState::Checking {
            top.append(
                &gtk::Spinner::builder()
                    .spinning(true)
                    .valign(gtk::Align::Center)
                    .build(),
            );
        }
        content.append(&top);
        if let Some(hint) = &entry.hint {
            content.append(&wrapped(hint, &["stave-edit-hint"]));
        }
        let mut description = entry.hint.clone().unwrap_or_default();
        match state {
            EntryState::Fits => {}
            EntryState::NeedsChoice(names) => {
                let words = needs_choice_text(names);
                content.append(&wrapped(&words, &["stave-edit-needs"]));
                description = format!("{description} {words}");
            }
            EntryState::Checking => {
                let words = "Checking whether it fits here…";
                content.append(&wrapped(words, &["stave-edit-checking"]));
                description = format!("{description} {words}");
            }
            EntryState::Greyed(reasons) => {
                let reason = reason_text(reasons);
                content.append(&wrapped(&reason, &["stave-edit-reason"]));
                description = format!("{description} Doesn't fit here: {reason}");
            }
        }
        let button = gtk::Button::builder()
            .child(&content)
            .css_classes(["stave-card-row", "flat", "stave-edit-entry"])
            .build();
        button.update_property(&[
            gtk::accessible::Property::Label(entry.label.as_str()),
            gtk::accessible::Property::Description(description.trim()),
        ]);
        let choosable = matches!(state, EntryState::Fits | EntryState::NeedsChoice(_));
        if choosable {
            button.set_tooltip_text(Some("Try it here"));
        } else {
            // Still focusable, so the reason can be read, but marked
            // unavailable and never reported.
            button.add_css_class("stave-edit-unavailable");
            button.update_state(&[gtk::accessible::State::Disabled(true)]);
        }
        let declaration = item.candidate.declaration;
        let on_chosen = self.on_chosen.clone();
        button.connect_clicked(move |_| {
            if !choosable {
                return;
            }
            let callback = on_chosen.borrow().clone();
            if let Some(callback) = callback {
                callback(declaration);
            }
        });
        button
    }

    /// The entries shown, in order, with their buttons (for tests).
    #[cfg(test)]
    pub fn rows(&self) -> Vec<(Declaration, gtk::Button)> {
        self.rows.borrow().clone()
    }
}

/// A wrapping label.
fn wrapped(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(classes.to_vec())
        .build()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use stave_app::edit::clothing_cabinet_fixture;
    use stave_lang::ast::{Ident, Name, Span};
    use stave_lang::{Code, Label, SourceMap};
    use stave_model::ids::ComponentId;

    fn renamed(candidate: &Candidate, label: &str, group: MenuGroup, id: usize) -> Candidate {
        let mut candidate = candidate.clone();
        candidate.entry.label = Ident::new(label, Span::default()).unwrap();
        candidate.entry.group = group;
        candidate.declaration = Declaration::Component(ComponentId::from_index(id).unwrap());
        candidate
    }

    pub(in crate::ui) fn greyed(candidate: &Candidate, message: &str) -> Candidate {
        let mut sources = SourceMap::new();
        let file = sources.add("greyed.stave", "x").unwrap();
        let diagnostic =
            Diagnostic::error(Code::Overlap, message, Label::new(file, Span::default()));
        let mut candidate = candidate.clone();
        candidate.applicability = Applicability::Greyed(NonEmpty::new(vec![diagnostic]).unwrap());
        candidate
    }

    #[test]
    fn groups_come_project_first_then_modules_then_standard() {
        let fixture = clothing_cabinet_fixture().unwrap();
        let drawer = &fixture.candidates[0];
        let name = |text: &str| Ident::new(text, Span::default()).unwrap();
        let candidates = [
            renamed(
                drawer,
                "Shelf",
                MenuGroup::Standard(Name::new("joinery", Span::default()).unwrap()),
                10,
            ),
            renamed(drawer, "Plinth", MenuGroup::Project, 11),
            renamed(drawer, "Drawer", MenuGroup::Module(name("casework")), 12),
            greyed(
                &renamed(drawer, "Apron", MenuGroup::Module(name("casework")), 13),
                "no room",
            ),
            renamed(drawer, "Bracket", MenuGroup::Module(name("casework")), 14),
        ];
        let mut items: Vec<MenuItem> = candidates
            .iter()
            .map(|candidate| MenuItem {
                candidate,
                checked: Checked::Done,
            })
            .collect();
        items[4].checked = Checked::Pending;
        let order: Vec<(String, Vec<&str>)> = sections(&items)
            .into_iter()
            .map(|(group, entries)| {
                (
                    group_title(group),
                    entries
                        .iter()
                        .map(|item| item.candidate.entry.label.as_str())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                ("In this project".to_owned(), vec!["Plinth"]),
                ("casework".to_owned(), vec!["Drawer", "Bracket", "Apron"]),
                ("Standard · joinery".to_owned(), vec!["Shelf"]),
            ]
        );
    }

    #[test]
    fn a_greyed_entry_says_the_checkers_reason() {
        let fixture = clothing_cabinet_fixture().unwrap();
        let door = greyed(
            &fixture.candidates[1],
            "door_l is already in door_bay[left]",
        );
        let item = MenuItem {
            candidate: &door,
            checked: Checked::Done,
        };
        let EntryState::Greyed(reasons) = EntryState::of(&item) else {
            panic!("greyed");
        };
        assert_eq!(reason_text(reasons), "door_l is already in door_bay[left]");
        let pending = MenuItem {
            checked: Checked::Pending,
            ..item
        };
        assert_eq!(EntryState::of(&pending), EntryState::Checking);
    }
}
