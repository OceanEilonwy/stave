//! The finder (Ctrl+K, V2C_Finder.dc.html): one field over the design's
//! parts, the project's files and names, its lines and its problems, with
//! a filter per kind. The hits come from `stave_app::finder::search`
//! through the window; this dialog only lists them and says which one was
//! chosen. Moving through the list previews a part in the 3D view; Enter
//! goes there; Ctrl+Enter opens its text in the editor.

use stave_app::i18n::{fill_in, tr};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::finder::{Hit, Kind};
use stave_lang::Severity;

type SearchCallback = Rc<dyn Fn(&str) -> Vec<Hit>>;
type HitCallback = Rc<dyn Fn(Hit, Choice)>;
type ActionsCallback = Rc<dyn Fn(&str) -> Vec<Action>>;
type ActionCallback = Rc<dyn Fn(usize)>;

/// Something the finder can do rather than go to: a verb typed in the
/// field ("add shelf", "remove", "rename door_left"), listed first under
/// "Do".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    /// What it does: "Add Shelf".
    pub label: String,
    /// Where or to what, dim after it: "in this space".
    pub detail: String,
}

/// What choosing a hit asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Arrow keys rested on it: show it, without going there.
    Preview,
    /// Enter or a click: go there.
    Go,
    /// Ctrl+Enter: open its text in the editor.
    OpenText,
}

#[derive(Default)]
struct State {
    hits: Vec<Hit>,
    /// The rows and the hit each stands for (headings stand for none).
    rows: Vec<(gtk::ListBoxRow, Option<usize>)>,
    filter: Option<Kind>,
    on_search: Option<SearchCallback>,
    on_hit: Option<HitCallback>,
    /// The verbs the query names, and their rows.
    actions: Vec<Action>,
    action_rows: Vec<(gtk::ListBoxRow, usize)>,
    on_actions: Option<ActionsCallback>,
    on_action: Option<ActionCallback>,
}

/// The finder dialog.
pub struct Finder {
    /// The dialog.
    pub dialog: adw::Dialog,
    entry: gtk::SearchEntry,
    filters: Vec<(Option<Kind>, gtk::ToggleButton)>,
    list: gtk::ListBox,
    state: RefCell<State>,
    /// Ignore selection changes made while filling.
    filling: Cell<bool>,
}

impl Finder {
    /// A closed finder.
    pub fn new() -> Rc<Finder> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text(tr("Find a part, file, line or problem"))
            .hexpand(true)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(&tr(
            "Find a part, file, line or problem",
        ))]);
        let filter_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-finder-filters"])
            .build();
        let mut filters = Vec::new();
        let all = gtk::ToggleButton::builder()
            .label(tr("All"))
            .active(true)
            .build();
        filter_row.append(&all);
        filters.push((None, all.clone()));
        for kind in Kind::ALL {
            let button = gtk::ToggleButton::builder()
                .label(kind.title())
                .group(&all)
                .build();
            filter_row.append(&button);
            filters.push((Some(kind), button));
        }
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["stave-finder-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&tr(
            "What the finder found",
        ))]);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .min_content_height(120)
            .max_content_height(480)
            .child(&list)
            .build();
        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-finder-foot"])
            .build();
        for words in [
            tr("↑ ↓ show it in 3D"),
            tr("Enter go there"),
            tr("Ctrl+Enter open the text"),
            tr("Esc close"),
        ] {
            foot.append(&gtk::Label::new(Some(&words)));
        }
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-finder"])
            .build();
        content.append(&entry);
        content.append(&filter_row);
        content.append(&scrolled);
        content.append(&foot);
        let dialog = adw::Dialog::builder()
            .child(&content)
            .content_width(640)
            .follows_content_size(true)
            .title(tr("Find"))
            .build();
        let finder = Rc::new(Finder {
            dialog,
            entry,
            filters,
            list,
            state: RefCell::new(State::default()),
            filling: Cell::new(false),
        });
        finder.connect();
        finder
    }

    /// Calls `callback` with the query to find what it matches.
    pub fn connect_search(&self, callback: impl Fn(&str) -> Vec<Hit> + 'static) {
        self.state.borrow_mut().on_search = Some(Rc::new(callback));
    }

    /// Calls `callback` with a hit and what was asked of it.
    pub fn connect_hit(&self, callback: impl Fn(Hit, Choice) + 'static) {
        self.state.borrow_mut().on_hit = Some(Rc::new(callback));
    }

    /// Calls `callback` with the query to list the verbs it names.
    pub fn connect_actions(&self, callback: impl Fn(&str) -> Vec<Action> + 'static) {
        self.state.borrow_mut().on_actions = Some(Rc::new(callback));
    }

    /// Calls `callback` with the index of the verb chosen (Enter or a
    /// click), once the finder has closed.
    pub fn connect_action(&self, callback: impl Fn(usize) + 'static) {
        self.state.borrow_mut().on_action = Some(Rc::new(callback));
    }

    /// Opens the finder over `parent`, with `query` typed (empty to start
    /// afresh).
    pub fn present(self: &Rc<Self>, parent: &impl IsA<gtk::Widget>, query: &str) {
        self.entry.set_text(query);
        self.refresh();
        self.dialog.present(Some(parent));
        self.entry.grab_focus();
    }

    fn connect(self: &Rc<Self>) {
        let finder = Rc::downgrade(self);
        self.entry.connect_search_changed(move |_| {
            if let Some(finder) = finder.upgrade() {
                finder.refresh();
            }
        });
        let finder = Rc::downgrade(self);
        self.entry.connect_activate(move |_| {
            if let Some(finder) = finder.upgrade() {
                finder.choose_selected(Choice::Go);
            }
        });
        for (kind, button) in &self.filters {
            let finder = Rc::downgrade(self);
            let kind = *kind;
            button.connect_toggled(move |button| {
                let Some(finder) = finder.upgrade() else {
                    return;
                };
                if button.is_active() {
                    finder.state.borrow_mut().filter = kind;
                    finder.fill();
                }
            });
        }
        let finder = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            if let Some(finder) = finder.upgrade() {
                finder.choose_row(row, Choice::Go);
            }
        });
        let finder = Rc::downgrade(self);
        self.list.connect_row_selected(move |_, row| {
            let (Some(finder), Some(row)) = (finder.upgrade(), row) else {
                return;
            };
            if finder.filling.get() {
                return;
            }
            finder.choose_row(row, Choice::Preview);
        });
        // Up and down in the field move through the list; Ctrl+Enter
        // opens the text; Tab moves to the next filter; Esc closes the
        // finder (the search field would only clear its text).
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let finder = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(finder) = finder.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
            match key {
                gtk::gdk::Key::Down => finder.step(1),
                gtk::gdk::Key::Up => finder.step(-1),
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter if control => {
                    finder.choose_selected(Choice::OpenText);
                }
                gtk::gdk::Key::Tab => finder.next_filter(),
                gtk::gdk::Key::Escape => {
                    finder.dialog.close();
                }
                _ => return gtk::glib::Propagation::Proceed,
            }
            gtk::glib::Propagation::Stop
        });
        self.entry.add_controller(keys);
    }

    /// Finds what the query matches again.
    fn refresh(&self) {
        let query = self.entry.text().to_string();
        let callback = self.state.borrow().on_search.clone();
        let hits = callback.map(|search| search(&query)).unwrap_or_default();
        let actions = self.state.borrow().on_actions.clone();
        let actions = actions.map(|list| list(&query)).unwrap_or_default();
        let mut state = self.state.borrow_mut();
        state.hits = hits;
        state.actions = actions;
        drop(state);
        self.show_counts();
        self.fill();
    }

    /// The filters' counts: "All 9", "Parts 4", ...
    fn show_counts(&self) {
        let state = self.state.borrow();
        for (kind, button) in &self.filters {
            let count = state
                .hits
                .iter()
                .filter(|hit| kind.is_none_or(|kind| hit.kind == kind))
                .count();
            let title = kind.map_or_else(|| tr("All"), Kind::title);
            button.set_label(&if state.hits.is_empty() {
                title
            } else {
                fill_in(
                    &tr("{group} {count}"),
                    &[("group", &title), ("count", &count)],
                )
            });
            button.set_sensitive(kind.is_none() || count > 0);
        }
    }

    /// Lists the hits the filter shows, under a heading per kind, and
    /// selects the first.
    fn fill(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let query = self.entry.text().to_string();
        let mut rows = Vec::new();
        let mut action_rows = Vec::new();
        {
            let state = self.state.borrow();
            if !state.actions.is_empty() && state.filter.is_none() {
                rows.push((action_heading(), None));
                for (index, action) in state.actions.iter().enumerate() {
                    let row = action_row(action);
                    action_rows.push((row.clone(), index));
                    rows.push((row, None));
                }
            }
            let mut last: Option<Kind> = None;
            for (index, hit) in state.hits.iter().enumerate() {
                if state.filter.is_some_and(|kind| kind != hit.kind) {
                    continue;
                }
                if last != Some(hit.kind) {
                    rows.push((heading(hit.kind), None));
                    last = Some(hit.kind);
                }
                rows.push((hit_row(hit), Some(index)));
            }
            if rows.is_empty() {
                let words = if query.trim().is_empty() {
                    tr(
                        // TRANSLATORS: the verbs are typed into the finder as
                        // written here, in English.
                        "Type a name: a part, a file, a word in the text or a problem; or a verb: add, remove, rename, repeat, mirror, hold",
                    )
                } else {
                    tr("Nothing matches that")
                };
                let label = gtk::Label::builder()
                    .label(words)
                    .css_classes(["dim-label"])
                    .margin_top(16)
                    .margin_bottom(16)
                    .build();
                rows.push((
                    gtk::ListBoxRow::builder()
                        .child(&label)
                        .activatable(false)
                        .selectable(false)
                        .build(),
                    None,
                ));
            }
        }
        self.filling.set(true);
        for (row, _) in &rows {
            self.list.append(row);
        }
        let first = action_rows.first().map(|(row, _)| row.clone()).or_else(|| {
            rows.iter()
                .find(|(_, hit)| hit.is_some())
                .map(|(row, _)| row.clone())
        });
        let mut state = self.state.borrow_mut();
        state.rows = rows;
        state.action_rows = action_rows;
        drop(state);
        match first {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        self.filling.set(false);
    }

    /// Moves the selection `by` rows of hits up or down.
    fn step(&self, by: i32) {
        let state = self.state.borrow();
        let hits: Vec<&gtk::ListBoxRow> = state
            .rows
            .iter()
            .filter(|(row, hit)| {
                hit.is_some() || state.action_rows.iter().any(|(action, _)| action == row)
            })
            .map(|(row, _)| row)
            .collect();
        let current = self
            .list
            .selected_row()
            .and_then(|selected| hits.iter().position(|row| **row == selected));
        let next = match (current, by.is_negative()) {
            (None, _) => Some(0),
            (Some(index), false) => Some(index.saturating_add(1)),
            (Some(index), true) => index.checked_sub(1),
        };
        if let Some(row) = next.and_then(|index| hits.get(index)) {
            let row = (*row).clone();
            drop(state);
            // The keyboard stays in the field; the list follows.
            self.list.select_row(Some(&row));
        }
    }

    /// Moves to the next filter that has hits (Tab).
    fn next_filter(&self) {
        let current = self
            .filters
            .iter()
            .position(|(_, button)| button.is_active())
            .unwrap_or(0);
        let count = self.filters.len();
        for step in 1..=count {
            let index = current.saturating_add(step) % count;
            if let Some((_, button)) = self.filters.get(index)
                && button.is_sensitive()
            {
                button.set_active(true);
                return;
            }
        }
    }

    fn choose_selected(&self, choice: Choice) {
        if let Some(row) = self.list.selected_row() {
            self.choose_row(&row, choice);
        }
    }

    fn choose_row(&self, row: &gtk::ListBoxRow, choice: Choice) {
        let action = {
            let state = self.state.borrow();
            state
                .action_rows
                .iter()
                .find(|(r, _)| r == row)
                .map(|(_, index)| (*index, state.on_action.clone()))
        };
        if let Some((index, callback)) = action {
            if choice == Choice::Preview {
                return;
            }
            self.dialog.close();
            if let Some(callback) = callback {
                callback(index);
            }
            return;
        }
        let (hit, callback) = {
            let state = self.state.borrow();
            let hit = state
                .rows
                .iter()
                .find(|(r, _)| r == row)
                .and_then(|(_, index)| state.hits.get((*index)?).cloned());
            (hit, state.on_hit.clone())
        };
        let (Some(hit), Some(callback)) = (hit, callback) else {
            return;
        };
        if choice != Choice::Preview {
            self.dialog.close();
        }
        callback(hit, choice);
    }
}

fn heading(kind: Kind) -> gtk::ListBoxRow {
    let words = match kind {
        Kind::Part => tr("Parts · Enter frames it in 3D"),
        Kind::File => tr("Files · Enter opens the text"),
        Kind::Name => tr("Names · Enter opens where it is written"),
        Kind::Line => tr("Lines"),
        Kind::Problem => tr("Problems · Enter shows where it happens"),
    };
    let label = gtk::Label::builder()
        .label(words)
        .xalign(0.0)
        .css_classes(["stave-finder-heading"])
        .build();
    if kind == Kind::Problem {
        label.add_css_class("problems");
    }
    gtk::ListBoxRow::builder()
        .child(&label)
        .activatable(false)
        .selectable(false)
        .build()
}

fn action_heading() -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(tr("Do · Enter does it"))
        .xalign(0.0)
        .css_classes(["stave-finder-heading"])
        .build();
    gtk::ListBoxRow::builder()
        .child(&label)
        .activatable(false)
        .selectable(false)
        .build()
}

fn action_row(action: &Action) -> gtk::ListBoxRow {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .css_classes(["stave-finder-hit"])
        .build();
    content.append(&gtk::Image::from_icon_name("go-next-symbolic"));
    content.append(
        &gtk::Label::builder()
            .label(&action.label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["heading"])
            .build(),
    );
    content.append(
        &gtk::Label::builder()
            .label(&action.detail)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .css_classes(["stave-detail"])
            .build(),
    );
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(&format!(
        "{} {}",
        action.label, action.detail
    ))]);
    row
}

fn hit_row(hit: &Hit) -> gtk::ListBoxRow {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["stave-finder-hit"])
        .build();
    if let Some(severity) = hit.severity {
        let mark = match severity {
            Severity::Error => Some(stave_app::problems::Mark::Error),
            Severity::Warning => Some(stave_app::problems::Mark::Warning),
            Severity::Note => None,
        };
        content.append(&super::style::mark_dot(mark));
    }
    let label = gtk::Label::builder()
        .label(&hit.label)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    if matches!(hit.kind, Kind::Line | Kind::File) {
        label.add_css_class("stave-finder-line");
    }
    if hit.kind == Kind::Line {
        // The place leads a line of text, as in the editor's gutter.
        content.append(
            &gtk::Label::builder()
                .label(&hit.detail)
                .width_chars(16)
                .xalign(0.0)
                .css_classes(["stave-detail", "stave-finder-line"])
                .build(),
        );
        content.append(&label);
    } else {
        content.append(&label);
        content.append(
            &gtk::Label::builder()
                .label(&hit.detail)
                .ellipsize(gtk::pango::EllipsizeMode::Start)
                .css_classes(["stave-detail"])
                .build(),
        );
    }
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(&format!(
        "{} {}",
        hit.label, hit.detail
    ))]);
    row
}
