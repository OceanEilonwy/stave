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
//!
//! Two drivers draw it with the same code: [`EditMenu`], the API the
//! window uses today, which draws at once and reports a choice from the
//! entry's own signal; and [`EditMenuComponent`], the relm4 component
//! the window will own once it is one (docs/relm4-migration.md), whose
//! entries only send it messages.

use std::rc::Rc;

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::edit::{Applicability, Candidate, Declaration, Heading, MenuGroup};
use stave_lang::Diagnostic;
use stave_lang::ast::Name;
use stave_model::nonempty::NonEmpty;

use super::edit_card::{Callback, install_style};

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

/// How an entry is drawn, in words the menu owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryLook {
    /// It applies: it can be chosen.
    Fits,
    /// It can be chosen, and the card then asks for more: "Then point at
    /// or choose: hinge side".
    NeedsChoice(String),
    /// Still being checked: it can't be chosen yet.
    Checking,
    /// It doesn't apply here: the checker's reasons, one a line.
    Greyed(String),
}

impl EntryLook {
    fn of(state: EntryState<'_>) -> EntryLook {
        match state {
            EntryState::Fits => EntryLook::Fits,
            EntryState::NeedsChoice(names) => EntryLook::NeedsChoice(needs_choice_text(names)),
            EntryState::Checking => EntryLook::Checking,
            EntryState::Greyed(reasons) => EntryLook::Greyed(reason_text(reasons)),
        }
    }

    /// `true` when the entry can be chosen.
    fn choosable(&self) -> bool {
        match self {
            EntryLook::Fits | EntryLook::NeedsChoice(_) => true,
            EntryLook::Checking | EntryLook::Greyed(_) => false,
        }
    }
}

/// One entry of the menu, owned, so it can travel in a message
/// ([`EditMenuInput::Show`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuEntry {
    /// What choosing it reports.
    pub declaration: Declaration,
    /// The words over its group: "In this project", "casework".
    pub group: String,
    /// Its label: "Drawer".
    pub label: String,
    /// The line under its label, if it has one.
    pub hint: Option<String>,
    /// How it is drawn.
    pub look: EntryLook,
}

impl MenuEntry {
    /// The entries as the menu lists them: grouped (the project's own
    /// first, then imported modules, then the standard ones), and within
    /// a group those that fit, then those being checked, then the greyed
    /// ones, each in name order.
    pub fn list(items: &[MenuItem<'_>]) -> Vec<MenuEntry> {
        sections(items)
            .into_iter()
            .flat_map(|(group, entries)| {
                let title = group_title(group);
                entries.into_iter().map(move |item| MenuEntry {
                    declaration: item.candidate.declaration,
                    group: title.clone(),
                    label: item.candidate.entry.label.as_str().to_owned(),
                    hint: item.candidate.entry.hint.clone(),
                    look: EntryLook::of(EntryState::of(item)),
                })
            })
            .collect()
    }
}

/// The menu's entries, in order, with their buttons.
type MenuRows = Vec<(Declaration, gtk::Button)>;

/// Fills the menu's heading and list for a selection of this kind with
/// these entries (in [`MenuEntry::list`] order). Each entry that can be
/// chosen calls `chosen` with its declaration when pressed. Returns the
/// entries' buttons, in order.
fn fill_menu(
    root: &gtk::Box,
    heading_label: &gtk::Label,
    list: &gtk::Box,
    heading: Heading,
    entries: &[MenuEntry],
    chosen: &(impl Fn(Declaration) + Clone + 'static),
) -> MenuRows {
    let heading = heading.text();
    heading_label.set_label(&heading);
    root.update_property(&[gtk::accessible::Property::Label(&heading)]);
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    if entries.is_empty() {
        list.append(
            &gtk::Label::builder()
                .label("Nothing in this project or its imports goes here.")
                .xalign(0.0)
                .wrap(true)
                .css_classes(["stave-description"])
                .build(),
        );
    }
    let mut rows = Vec::with_capacity(entries.len());
    let mut rest = entries;
    while let Some(first) = rest.first() {
        let count = rest
            .iter()
            .take_while(|entry| entry.group == first.group)
            .count();
        let (group, after) = rest.split_at(count);
        rest = after;
        let section = gtk::Box::new(gtk::Orientation::Vertical, 8);
        section.append(
            &gtk::Label::builder()
                .label(first.group.to_uppercase())
                .xalign(0.0)
                .css_classes(["stave-section"])
                .build(),
        );
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-card"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        for entry in group {
            let button = entry_button(entry, chosen);
            card.append(&button);
            rows.push((entry.declaration, button));
        }
        section.append(&card);
        list.append(&section);
    }
    rows
}

/// One entry: its label, its hint and, when it can't be chosen, why.
fn entry_button(
    entry: &MenuEntry,
    chosen: &(impl Fn(Declaration) + Clone + 'static),
) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    top.append(
        &gtk::Label::builder()
            .label(&entry.label)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["stave-edit-entry-label"])
            .build(),
    );
    if entry.look == EntryLook::Checking {
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
    match &entry.look {
        EntryLook::Fits => {}
        EntryLook::NeedsChoice(words) => {
            content.append(&wrapped(words, &["stave-edit-needs"]));
            description = format!("{description} {words}");
        }
        EntryLook::Checking => {
            let words = "Checking whether it fits here…";
            content.append(&wrapped(words, &["stave-edit-checking"]));
            description = format!("{description} {words}");
        }
        EntryLook::Greyed(reason) => {
            content.append(&wrapped(reason, &["stave-edit-reason"]));
            description = format!("{description} Doesn't fit here: {reason}");
        }
    }
    let button = gtk::Button::builder()
        .child(&content)
        .css_classes(["stave-card-row", "flat", "stave-edit-entry"])
        .build();
    button.update_property(&[
        gtk::accessible::Property::Label(&entry.label),
        gtk::accessible::Property::Description(description.trim()),
    ]);
    if entry.look.choosable() {
        button.set_tooltip_text(Some("Try it here"));
        let declaration = entry.declaration;
        let chosen = chosen.clone();
        button.connect_clicked(move |_| chosen(declaration));
    } else {
        // Still focusable, so the reason can be read, but marked
        // unavailable and never reported.
        button.add_css_class("stave-edit-unavailable");
        button.update_state(&[gtk::accessible::State::Disabled(true)]);
    }
    button
}

/// The menu's panel, ready to float over the 3D view; empty until
/// [`fill_frame`] puts the heading and the list in it.
fn menu_root() -> gtk::Box {
    if let Some(display) = gtk::gdk::Display::default() {
        install_style(&display);
    }
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(["stave-card-panel", "stave-edit-menu"])
        .width_request(320)
        .accessible_role(gtk::AccessibleRole::Group)
        .overflow(gtk::Overflow::Hidden)
        .build()
}

/// Puts the heading and the list, in a scrolled box, in the menu's
/// panel. Returns the heading and the list.
fn fill_frame(root: &gtk::Box) -> (gtk::Label, gtk::Box) {
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
    root.append(&scrolled);
    (heading, list)
}

/// The menu for a selection, behind the API the window still uses: what
/// it shows is drawn at once and a choice is reported at once, from the
/// entry's own signal, because the window's code and its tests read the
/// menu straight after showing it. [`EditMenuComponent`] is the same menu
/// as a relm4 component, for the window to own once it is one
/// (docs/relm4-migration.md).
pub struct EditMenu {
    /// The panel, ready to float over the 3D view.
    pub root: gtk::Box,
    heading: gtk::Label,
    list: gtk::Box,
    /// The entries shown, in order, with their buttons (for tests).
    #[cfg(test)]
    rows: std::cell::Cell<MenuRows>,
    on_chosen: Rc<Callback<Declaration>>,
}

impl EditMenu {
    /// An empty menu.
    pub fn new() -> EditMenu {
        let root = menu_root();
        let (heading, list) = fill_frame(&root);
        EditMenu {
            root,
            heading,
            list,
            #[cfg(test)]
            rows: std::cell::Cell::default(),
            on_chosen: Rc::default(),
        }
    }

    /// Calls `callback` with the declaration of the entry the user chose.
    /// Greyed entries and entries still being checked never call it.
    pub fn connect_chosen(&self, callback: impl Fn(Declaration) + 'static) {
        self.on_chosen.connect(callback);
    }

    /// Shows the menu for a selection of this kind with these entries.
    pub fn show(&self, heading: Heading, items: &[MenuItem<'_>]) {
        let on_chosen = Rc::clone(&self.on_chosen);
        let rows = fill_menu(
            &self.root,
            &self.heading,
            &self.list,
            heading,
            &MenuEntry::list(items),
            &move |declaration| on_chosen.call(declaration),
        );
        #[cfg(test)]
        self.rows.set(rows);
        #[cfg(not(test))]
        let _ = rows;
    }

    /// The entries shown, in order, with their buttons (for tests).
    #[cfg(test)]
    pub fn rows(&self) -> MenuRows {
        let rows = self.rows.take();
        self.rows.set(rows.clone());
        rows
    }
}

/// The menu for a selection as a relm4 component: its model is the
/// entries' buttons, only [`EditMenuComponent::update_with_view`] draws
/// them, and an entry's button only sends [`EditMenuInput::Chose`].
#[derive(Debug, Default)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the relm4 menu the window will own once it is a component; until then only tests launch it (docs/relm4-migration.md)"
    )
)]
pub struct EditMenuComponent {
    /// The entries shown, in order, with their buttons.
    rows: MenuRows,
}

/// What the menu is told.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the relm4 menu the window will own once it is a component; until then only tests launch it (docs/relm4-migration.md)"
    )
)]
pub enum EditMenuInput {
    /// Show the menu for a selection of this kind with these entries (in
    /// [`MenuEntry::list`] order).
    Show {
        /// The kind of selection.
        heading: Heading,
        /// The entries.
        items: Vec<MenuEntry>,
    },
    /// People pressed an entry that can be chosen.
    Chose(Declaration),
}

/// What the menu tells its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the relm4 menu the window will own once it is a component; until then only tests launch it (docs/relm4-migration.md)"
    )
)]
pub enum EditMenuOutput {
    /// People chose the entry for this declaration.
    Chosen(Declaration),
}

/// The menu component's widgets.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the relm4 menu the window will own once it is a component; until then only tests launch it (docs/relm4-migration.md)"
    )
)]
pub struct EditMenuWidgets {
    heading: gtk::Label,
    list: gtk::Box,
}

impl Component for EditMenuComponent {
    type Init = ();
    type Input = EditMenuInput;
    type Output = EditMenuOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = EditMenuWidgets;

    fn init_root() -> gtk::Box {
        menu_root()
    }

    fn init(_: (), root: gtk::Box, _: ComponentSender<Self>) -> ComponentParts<Self> {
        let (heading, list) = fill_frame(&root);
        ComponentParts {
            model: EditMenuComponent::default(),
            widgets: EditMenuWidgets { heading, list },
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut EditMenuWidgets,
        message: EditMenuInput,
        sender: ComponentSender<Self>,
        root: &gtk::Box,
    ) {
        match message {
            EditMenuInput::Show { heading, items } => {
                let input = sender.input_sender().clone();
                self.rows = fill_menu(
                    root,
                    &widgets.heading,
                    &widgets.list,
                    heading,
                    &items,
                    &move |declaration| input.emit(EditMenuInput::Chose(declaration)),
                );
            }
            EditMenuInput::Chose(declaration) => {
                // Nobody listening is not a mistake.
                let _ = sender.output(EditMenuOutput::Chosen(declaration));
            }
        }
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
    use crate::ui::relm4_shim::Bridge;
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

    /// The menu component's entries as the fixture's first two
    /// candidates make them: Drawer fits, the door is still checked.
    fn fixture_entries() -> (Heading, Vec<MenuEntry>, Declaration) {
        let fixture = clothing_cabinet_fixture().unwrap();
        let items: Vec<MenuItem> = fixture
            .candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| MenuItem {
                candidate,
                checked: if index == 0 {
                    Checked::Done
                } else {
                    Checked::Pending
                },
            })
            .collect();
        (
            fixture.picked.heading(),
            MenuEntry::list(&items),
            fixture.candidates[0].declaration,
        )
    }

    #[test]
    fn entries_are_owned_in_the_order_the_menu_lists_them() {
        let (_, entries, drawer) = fixture_entries();
        assert_eq!(entries[0].declaration, drawer);
        assert_eq!(entries[0].look, EntryLook::Fits);
        assert_eq!(entries[1].look, EntryLook::Checking);
        assert!(entries[0].look.choosable() && !entries[1].look.choosable());
    }

    #[test]
    fn a_dropped_menu_component_is_freed() {
        crate::ui::gtk_test::on_gtk_thread(menu_component_is_freed);
    }

    /// Dropping the bridge shuts the component down when the main loop
    /// next runs: its widget, its model (with the entries' buttons) and
    /// the callback are freed.
    fn menu_component_is_freed() {
        use crate::ui::gtk_test::settle;
        let menu = Bridge::<EditMenuComponent>::launch(());
        let (heading, items, _) = fixture_entries();
        menu.send(EditMenuInput::Show { heading, items });
        let token = Rc::new(());
        let held = Rc::clone(&token);
        menu.connect(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        settle();
        let root = menu.widget().downgrade();
        let button = menu.model().rows[0].1.downgrade();
        let input = menu.sender().clone();
        drop(menu);
        settle();
        assert!(root.upgrade().is_none(), "the panel is freed");
        assert!(button.upgrade().is_none(), "the entries are freed");
        assert!(
            input
                .send(EditMenuInput::Show {
                    heading: Heading::InThisSpace,
                    items: Vec::new()
                })
                .is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn showing_the_menu_component_draws_it_and_reports_nothing() {
        crate::ui::gtk_test::on_gtk_thread(menu_component_shows);
    }

    fn menu_component_shows() {
        use crate::ui::gtk_test::settle;
        let menu = Bridge::<EditMenuComponent>::launch(());
        let heard: Rc<std::cell::RefCell<Vec<EditMenuOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        menu.connect(move |output| log.borrow_mut().push(output));
        let (heading, items, drawer) = fixture_entries();
        let count = items.len();
        menu.send(EditMenuInput::Show { heading, items });
        assert!(menu.model().rows.is_empty(), "only queued");
        settle();
        let rows = menu.model().rows.clone();
        assert_eq!(rows.len(), count);
        assert_eq!(rows[0].0, drawer);
        assert!(heard.borrow().is_empty(), "showing reports nothing");
        // An entry still being checked never reports.
        rows[1].1.emit_clicked();
        settle();
        assert!(heard.borrow().is_empty());
    }

    #[test]
    fn a_choice_made_while_one_is_being_heard_is_queued_not_re_entered() {
        crate::ui::gtk_test::on_gtk_thread(menu_choices_are_queued);
    }

    /// The old menu called the window from inside the entry's signal; a
    /// window that redrew the menu from there rebuilt the list under the
    /// button still being pressed. Now a press only queues a message, the
    /// window hears the choice after the update has returned (reading the
    /// model there would panic if it had not), and a redraw and another
    /// press from inside the callback are queued too.
    fn menu_choices_are_queued() {
        use crate::ui::gtk_test::settle;
        let menu = Rc::new(Bridge::<EditMenuComponent>::launch(()));
        let (heading, items, drawer) = fixture_entries();
        menu.send(EditMenuInput::Show {
            heading,
            items: items.clone(),
        });
        settle();
        // (what was heard, how many entries the model held then)
        let heard: Rc<std::cell::RefCell<Vec<(EditMenuOutput, usize)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&menu);
        menu.connect(move |output| {
            let Some(menu) = weak.upgrade() else {
                return;
            };
            let rows = menu.model().rows.clone();
            let first = log.borrow().is_empty();
            log.borrow_mut().push((output, rows.len()));
            if first {
                // Redrawn from here: queued, so the list stays put now.
                menu.send(EditMenuInput::Show {
                    heading,
                    items: items[..1].to_vec(),
                });
                // And a press from in here is only queued.
                rows[0].1.emit_clicked();
                assert_eq!(menu.model().rows.len(), rows.len(), "not redrawn yet");
            }
        });
        let button = menu.model().rows[0].1.clone();
        button.emit_clicked();
        assert!(heard.borrow().is_empty(), "the press is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                (EditMenuOutput::Chosen(drawer), 2),
                (EditMenuOutput::Chosen(drawer), 1)
            ],
            "each choice after its update, the inner one after the redraw"
        );
    }
}
