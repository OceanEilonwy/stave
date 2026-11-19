//! The finder (Ctrl+K, V2C_Finder.dc.html): one field over the design's
//! parts, the project's files and names, its lines and its problems, with
//! a filter per kind. The hits come from `stave_app::finder::search`
//! through the window; this dialog only lists them and says which one was
//! chosen. Moving through the list previews a part in the 3D view; Enter
//! goes there; Ctrl+Enter opens its text in the editor.
//!
//! It is a relm4 component, [`FinderComponent`] (docs/relm4-migration.md):
//! its model is the only state, only its update changes it, and its
//! widgets' handlers only send [`FinderInput`] messages. What a query
//! matches is a request and a reply: the finder asks with
//! [`FinderOutput::Search`] and [`FinderOutput::SearchActions`] and lists
//! what comes back in [`FinderInput::Hits`] and [`FinderInput::Actions`].
//! [`Finder`] keeps the older callback API in front of it, answering those
//! requests with the window's callbacks, until the window moves to relm4
//! too.

use adw::prelude::*;
use gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::finder::{Hit, Kind};
use stave_app::i18n::{fill_in, tr};
use stave_lang::Severity;

use super::relm4_shim::Bridge;

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

/// Which way the arrow keys move through the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// ↑: the hit above.
    Up,
    /// ↓: the hit below.
    Down,
}

/// What a row of the list stands for, in the list's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    /// A heading over a group, or the hint when nothing is listed.
    Words,
    /// The verb at this index of the actions.
    Action(usize),
    /// The hit at this index of the hits.
    Hit(usize),
}

/// How many replies the finder still waits for: one per request, so the
/// list is filled once, from the replies to the latest query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Waiting {
    hits: usize,
    actions: usize,
}

/// The finder's state: the query, what it matched, the filter and what
/// each row of the list stands for.
#[derive(Debug, Default)]
pub struct FinderComponent {
    /// The query last asked about.
    query: String,
    hits: Vec<Hit>,
    /// The verbs the query names.
    actions: Vec<Action>,
    filter: Option<Kind>,
    rows: Vec<Row>,
    waiting: Waiting,
    /// Whether the owner listens for chosen hits
    /// ([`FinderInput::ListenForHits`]): until it does, choosing a hit
    /// does nothing and the finder stays open.
    hits_heard: bool,
}

/// A filter's toggle and the kind it shows (`None`: all of them).
#[derive(Debug)]
struct Filter {
    kind: Option<Kind>,
    button: gtk::ToggleButton,
}

/// What the finder is told.
#[derive(Debug)]
pub enum FinderInput {
    /// Open over `parent` with `query` typed (empty to start afresh).
    Present {
        /// The widget it opens over.
        parent: gtk::Widget,
        /// What is typed in its field.
        query: String,
    },
    /// The reply to [`FinderOutput::Search`]: what the query matches.
    Hits(Vec<Hit>),
    /// The reply to [`FinderOutput::SearchActions`]: the verbs it names.
    Actions(Vec<Action>),
    /// People typed: the field says this now.
    Typed(String),
    /// People switched on the filter for this kind (`None`: all).
    Filter(Option<Kind>),
    /// People pressed ↑ or ↓ in the field.
    Step(Direction),
    /// People pressed Tab: the next filter that has hits.
    NextFilter,
    /// People pressed Enter (Go) or Ctrl+Enter (OpenText) in the field.
    ChooseSelected(Choice),
    /// People clicked a row, or pressed Enter on it.
    RowActivated(gtk::ListBoxRow),
    /// The selection moved to a row (the arrow keys move it).
    RowSelected(gtk::ListBoxRow),
    /// People pressed Escape.
    Close,
    /// The owner now listens for [`FinderOutput::Hit`]. Until it does,
    /// choosing a hit does nothing: the finder neither closes nor tells
    /// anyone.
    ListenForHits,
}

/// What the finder tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinderOutput {
    /// What does this query match? Answer with [`FinderInput::Hits`].
    Search(String),
    /// Which verbs does this query name? Answer with
    /// [`FinderInput::Actions`].
    SearchActions(String),
    /// A hit was chosen; for [`Choice::Go`] and [`Choice::OpenText`] the
    /// finder has closed.
    Hit {
        /// The hit.
        hit: Hit,
        /// What was asked of it.
        choice: Choice,
    },
    /// The verb at this index of the last [`FinderInput::Actions`] was
    /// chosen, and the finder has closed.
    Action(usize),
}

impl FinderComponent {
    /// Asks what `query` matches; the list is filled when both replies
    /// are in.
    fn ask(&mut self, query: String, sender: &ComponentSender<Self>) {
        self.query = query.clone();
        self.waiting.hits = self.waiting.hits.saturating_add(1);
        self.waiting.actions = self.waiting.actions.saturating_add(1);
        // Nobody listening is not a mistake: the owner may not want to
        // hear. (Then no reply comes, and the list stays as it is.)
        let _ = sender.output(FinderOutput::Search(query.clone()));
        let _ = sender.output(FinderOutput::SearchActions(query));
    }

    /// The filters' counts: "All 9", "Parts 4", ...
    fn show_counts(&self, filters: &[Filter]) {
        for Filter { kind, button } in filters {
            let count = self
                .hits
                .iter()
                .filter(|hit| kind.is_none_or(|kind| hit.kind == kind))
                .count();
            let title = kind.map_or_else(|| tr("All"), Kind::title);
            button.set_label(&if self.hits.is_empty() {
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
    /// selects the first, without previewing it.
    fn fill(&mut self, widgets: &FinderComponentWidgets) {
        let list = &widgets.list;
        while let Some(row) = list.row_at_index(0) {
            list.remove(&row);
        }
        let mut rows: Vec<(gtk::ListBoxRow, Row)> = Vec::new();
        if !self.actions.is_empty() && self.filter.is_none() {
            rows.push((action_heading(), Row::Words));
            for (index, action) in self.actions.iter().enumerate() {
                rows.push((action_row(action), Row::Action(index)));
            }
        }
        let mut last: Option<Kind> = None;
        for (index, hit) in self.hits.iter().enumerate() {
            if self.filter.is_some_and(|kind| kind != hit.kind) {
                continue;
            }
            if last != Some(hit.kind) {
                rows.push((heading(hit.kind), Row::Words));
                last = Some(hit.kind);
            }
            rows.push((hit_row(hit), Row::Hit(index)));
        }
        if rows.is_empty() {
            let words = if self.query.trim().is_empty() {
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
            let hint = gtk::ListBoxRow::builder()
                .child(&label)
                .activatable(false)
                .selectable(false)
                .build();
            rows.push((hint, Row::Words));
        }
        // The first verb, or else the first hit.
        let first = rows
            .iter()
            .find(|(_, row)| matches!(row, Row::Action(_)))
            .or_else(|| rows.iter().find(|(_, row)| matches!(row, Row::Hit(_))))
            .map(|(widget, _)| widget.clone());
        self.rows = rows.iter().map(|(_, row)| *row).collect();
        // Selecting while filling previews nothing.
        list.block_signal(&widgets.row_selected);
        for (widget, _) in &rows {
            list.append(widget);
        }
        match first {
            Some(row) => list.select_row(Some(&row)),
            None => list.unselect_all(),
        }
        list.unblock_signal(&widgets.row_selected);
    }

    /// What a row of the list stands for, if it is still in the list.
    fn row_of(&self, list: &gtk::ListBox, row: &gtk::ListBoxRow) -> Option<Row> {
        if row.parent().as_ref() != Some(list.upcast_ref()) {
            return None;
        }
        usize::try_from(row.index())
            .ok()
            .and_then(|index| self.rows.get(index))
            .copied()
    }

    /// Moves the selection one hit (or verb) up or down; the keyboard
    /// stays in the field, the list follows, and the selection previews.
    fn step(&self, list: &gtk::ListBox, direction: Direction) {
        let choosable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, Row::Action(_) | Row::Hit(_)))
            .map(|(index, _)| index)
            .collect();
        let current = list
            .selected_row()
            .and_then(|row| usize::try_from(row.index()).ok())
            .and_then(|index| choosable.iter().position(|row| *row == index));
        let next = match (current, direction) {
            (None, Direction::Up | Direction::Down) => Some(0),
            (Some(at), Direction::Down) => Some(at.saturating_add(1)),
            (Some(at), Direction::Up) => at.checked_sub(1),
        };
        let row = next
            .and_then(|at| choosable.get(at))
            .and_then(|index| i32::try_from(*index).ok())
            .and_then(|index| list.row_at_index(index));
        if let Some(row) = row {
            list.select_row(Some(&row));
        }
    }

    /// Chooses what a row stands for: a verb closes the finder and runs
    /// (resting on one does nothing); a hit is previewed, or the finder
    /// closes and goes there. With nobody listening for hits, a hit does
    /// nothing and the finder stays open.
    fn choose(
        &self,
        row: Row,
        choice: Choice,
        dialog: &adw::Dialog,
        sender: &ComponentSender<Self>,
    ) {
        match row {
            Row::Words => {}
            Row::Action(index) => match choice {
                Choice::Preview => {}
                Choice::Go | Choice::OpenText => {
                    dialog.close();
                    let _ = sender.output(FinderOutput::Action(index));
                }
            },
            Row::Hit(index) => {
                let Some(hit) = self.hits.get(index).cloned().filter(|_| self.hits_heard) else {
                    return;
                };
                match choice {
                    Choice::Preview => {}
                    Choice::Go | Choice::OpenText => {
                        dialog.close();
                    }
                }
                let _ = sender.output(FinderOutput::Hit { hit, choice });
            }
        }
    }
}

/// Moves to the next filter that has hits (Tab); switching it on sends
/// [`FinderInput::Filter`], as a click would.
fn next_filter(filters: &[Filter]) {
    let current = filters
        .iter()
        .position(|filter| filter.button.is_active())
        .unwrap_or(0);
    let count = filters.len();
    for step in 1..=count {
        let index = current.saturating_add(step) % count;
        if let Some(filter) = filters.get(index)
            && filter.button.is_sensitive()
        {
            filter.button.set_active(true);
            return;
        }
    }
}

/// The input a key pressed in the field asks for, if any: ↑ and ↓ move
/// through the list, Ctrl+Enter opens the text, Tab moves to the next
/// filter and Escape closes the finder (the field would only clear its
/// text).
fn key_input(key: gdk::Key, modifiers: gdk::ModifierType) -> Option<FinderInput> {
    let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
    match key {
        gdk::Key::Down => Some(FinderInput::Step(Direction::Down)),
        gdk::Key::Up => Some(FinderInput::Step(Direction::Up)),
        gdk::Key::Return | gdk::Key::KP_Enter if control => {
            Some(FinderInput::ChooseSelected(Choice::OpenText))
        }
        gdk::Key::Tab => Some(FinderInput::NextFilter),
        gdk::Key::Escape => Some(FinderInput::Close),
        _ => None,
    }
}

#[relm4::component(pub)]
impl Component for FinderComponent {
    type Init = ();
    type Input = FinderInput;
    type Output = FinderOutput;
    type CommandOutput = ();

    view! {
        adw::Dialog {
            set_content_width: 640,
            set_follows_content_size: true,
            set_title: &tr("Find"),

            #[wrap(Some)]
            set_child = &gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                add_css_class: "stave-finder",

                #[name = "entry"]
                gtk::SearchEntry {
                    // What the field says before anything is typed, and
                    // what screen readers call it.
                    set_placeholder_text: Some(&tr("Find a part, file, line or problem")),
                    set_hexpand: true,
                    update_property: &[gtk::accessible::Property::Label(&tr(
                        "Find a part, file, line or problem",
                    ))],
                    connect_search_changed[sender] => move |entry| {
                        sender.input_sender().emit(FinderInput::Typed(entry.text().into()));
                    },
                    connect_activate[sender] => move |_| {
                        sender.input_sender().emit(FinderInput::ChooseSelected(Choice::Go));
                    },
                    add_controller = gtk::EventControllerKey {
                        set_propagation_phase: gtk::PropagationPhase::Capture,
                        connect_key_pressed[sender] => move |_, key, _, modifiers| {
                            let Some(input) = key_input(key, modifiers) else {
                                return glib::Propagation::Proceed;
                            };
                            sender.input_sender().emit(input);
                            glib::Propagation::Stop
                        },
                    },
                },

                #[local_ref]
                filter_row -> gtk::Box {
                    add_css_class: "stave-finder-filters",
                },

                gtk::ScrolledWindow {
                    set_hscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_height: true,
                    set_min_content_height: 120,
                    set_max_content_height: 480,

                    #[name = "list"]
                    gtk::ListBox {
                        set_selection_mode: gtk::SelectionMode::Single,
                        add_css_class: "stave-finder-list",
                        update_property: &[gtk::accessible::Property::Label(&tr(
                            "What the finder found",
                        ))],
                        connect_row_activated[sender] => move |_, row| {
                            sender.input_sender().emit(FinderInput::RowActivated(row.clone()));
                        },
                        connect_row_selected[sender] => move |_, row| {
                            if let Some(row) = row {
                                sender.input_sender().emit(FinderInput::RowSelected(row.clone()));
                            }
                        } @row_selected,
                    },
                },

                gtk::Box {
                    add_css_class: "stave-finder-foot",
                    gtk::Label { set_label: &tr("↑ ↓ show it in 3D") },
                    gtk::Label { set_label: &tr("Enter go there") },
                    gtk::Label { set_label: &tr("Ctrl+Enter open the text") },
                    gtk::Label { set_label: &tr("Esc close") },
                },
            },
        }
    }

    additional_fields! {
        filters: Vec<Filter>,
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = FinderComponent::default();
        // A filter for all, then one per kind, in one group.
        let filter_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let all = gtk::ToggleButton::builder()
            .label(tr("All"))
            .active(true)
            .build();
        let mut filters = vec![Filter {
            kind: None,
            button: all.clone(),
        }];
        filters.extend(Kind::ALL.into_iter().map(|kind| {
            Filter {
                kind: Some(kind),
                button: gtk::ToggleButton::builder()
                    .label(kind.title())
                    .group(&all)
                    .build(),
            }
        }));
        for Filter { kind, button } in &filters {
            let kind = *kind;
            let sender = sender.clone();
            button.connect_toggled(move |button| {
                if button.is_active() {
                    sender.input_sender().emit(FinderInput::Filter(kind));
                }
            });
            filter_row.append(button);
        }
        let filter_row = &filter_row;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: FinderInput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            FinderInput::Present { parent, query } => {
                // The field's own "search-changed" for this text says what
                // is already asked, and is let go.
                widgets.entry.set_text(&query);
                self.ask(query, &sender);
                root.present(Some(&parent));
                widgets.entry.grab_focus();
            }
            FinderInput::Typed(query) => {
                if query != self.query {
                    self.ask(query, &sender);
                }
            }
            FinderInput::Hits(hits) => {
                self.hits = hits;
                self.waiting.hits = self.waiting.hits.saturating_sub(1);
                if self.waiting == Waiting::default() {
                    self.show_counts(&widgets.filters);
                    self.fill(widgets);
                }
            }
            FinderInput::Actions(actions) => {
                self.actions = actions;
                self.waiting.actions = self.waiting.actions.saturating_sub(1);
                if self.waiting == Waiting::default() {
                    self.show_counts(&widgets.filters);
                    self.fill(widgets);
                }
            }
            FinderInput::Filter(kind) => {
                self.filter = kind;
                self.fill(widgets);
            }
            FinderInput::Step(direction) => self.step(&widgets.list, direction),
            FinderInput::NextFilter => next_filter(&widgets.filters),
            FinderInput::ChooseSelected(choice) => {
                let row = widgets
                    .list
                    .selected_row()
                    .and_then(|row| self.row_of(&widgets.list, &row));
                if let Some(row) = row {
                    self.choose(row, choice, root, &sender);
                }
            }
            FinderInput::RowActivated(row) => {
                if let Some(row) = self.row_of(&widgets.list, &row) {
                    self.choose(row, Choice::Go, root, &sender);
                }
            }
            FinderInput::RowSelected(row) => {
                if let Some(row) = self.row_of(&widgets.list, &row) {
                    self.choose(row, Choice::Preview, root, &sender);
                }
            }
            FinderInput::Close => {
                root.close();
            }
            FinderInput::ListenForHits => self.hits_heard = true,
        }
        self.update_view(widgets, sender);
    }
}

/// The finder dialog: it owns the component (dropping it shuts the
/// component down), passes [`Finder::present`] on as
/// [`FinderInput::Present`] and its [`FinderOutput`]s to the callback
/// connected with [`Finder::connect_outputs`]. The owner answers the
/// finder's searches from its own update with [`Finder::show_hits`] and
/// [`Finder::show_actions`] (docs/relm4-migration.md, rule 9).
///
/// Everything is queued: the finder opens, lists and tells its owner when
/// the main loop next runs, after whatever called it has returned.
pub struct Finder {
    /// The dialog.
    pub dialog: adw::Dialog,
    bridge: Bridge<FinderComponent>,
}

impl Finder {
    /// A closed finder.
    pub fn new() -> Finder {
        let bridge = Bridge::<FinderComponent>::launch(());
        Finder {
            dialog: bridge.widget().clone(),
            bridge,
        }
    }

    /// Calls `callback` with each request and choice from now on, instead
    /// of the one connected before. Until one is connected, choosing a
    /// hit leaves the finder open, as it always did.
    pub fn connect_outputs(&self, callback: impl Fn(FinderOutput) + 'static) {
        self.bridge.connect(callback);
        self.bridge.send(FinderInput::ListenForHits);
    }

    /// Lists `hits`: the answer to the last [`FinderOutput::Search`].
    pub fn show_hits(&self, hits: Vec<Hit>) {
        self.bridge.send(FinderInput::Hits(hits));
    }

    /// Lists `actions`: the answer to the last
    /// [`FinderOutput::SearchActions`].
    pub fn show_actions(&self, actions: Vec<Action>) {
        self.bridge.send(FinderInput::Actions(actions));
    }

    /// Opens the finder over `parent`, with `query` typed (empty to start
    /// afresh).
    pub fn present(&self, parent: &impl IsA<gtk::Widget>, query: &str) {
        self.bridge.send(FinderInput::Present {
            parent: parent.clone().upcast(),
            query: query.to_owned(),
        });
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

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use stave_app::finder::Goal;
    use stave_app::session::{LineNumber, SourceLocation};

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// Answers the finder's searches at once with `search` and `actions`,
    /// as the window does from its update, and passes its choices to
    /// `chosen`. Without `hits_heard`, the finder is not told anyone
    /// listens for hits.
    fn answer(
        finder: &Rc<Finder>,
        search: impl Fn(&str) -> Vec<Hit> + 'static,
        actions: impl Fn(&str) -> Vec<Action> + 'static,
        chosen: impl Fn(FinderOutput) + 'static,
        hits_heard: bool,
    ) {
        let weak = Rc::downgrade(finder);
        let callback = move |output| {
            let Some(finder) = weak.upgrade() else {
                return;
            };
            match output {
                FinderOutput::Search(query) => finder.show_hits(search(&query)),
                FinderOutput::SearchActions(query) => finder.show_actions(actions(&query)),
                FinderOutput::Hit { .. } | FinderOutput::Action(_) => chosen(output),
            }
        };
        if hits_heard {
            finder.connect_outputs(callback);
        } else {
            finder.bridge.connect(callback);
        }
    }

    /// The hit and its choice in a [`FinderOutput::Hit`].
    fn hit_of(output: FinderOutput) -> Option<(String, Choice)> {
        match output {
            FinderOutput::Hit { hit, choice } => Some((hit.label, choice)),
            FinderOutput::Search(_) | FinderOutput::SearchActions(_) | FinderOutput::Action(_) => {
                None
            }
        }
    }

    /// A file the finder found.
    fn file(label: &str) -> Hit {
        Hit {
            kind: Kind::File,
            label: label.to_owned(),
            detail: String::new(),
            goal: Goal::Open(SourceLocation {
                path: format!("{label}.stave"),
                line: LineNumber::new(1).expect("line 1"),
            }),
            severity: None,
        }
    }

    /// The first widget of type `T` in the finder's dialog.
    fn find<T: IsA<gtk::Widget>>(finder: &Finder) -> T {
        let mut widgets: Vec<gtk::Widget> = finder.dialog.child().into_iter().collect();
        while let Some(widget) = widgets.pop() {
            if let Ok(found) = widget.clone().downcast::<T>() {
                return found;
            }
            let mut child = widget.first_child();
            while let Some(each) = child {
                child = each.next_sibling();
                widgets.push(each);
            }
        }
        panic!("the finder has a {}", T::static_type().name());
    }

    /// A window to open the finder over, shown.
    fn window() -> (gtk::Window, gtk::Box) {
        let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let window = gtk::Window::builder()
            .default_width(800)
            .default_height(600)
            .child(&parent)
            .build();
        window.present();
        settle();
        (window, parent)
    }

    #[test]
    fn a_dropped_finder_is_freed() {
        on_gtk_thread(finder_is_freed);
    }

    /// Dropping the finder drops its controller, which shuts the
    /// component down when the main loop next runs: the dialog, the
    /// model (with the input channel the runtime held) and the window's
    /// callbacks are all freed.
    fn finder_is_freed() {
        let finder = Finder::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        finder.connect_outputs(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let dialog = finder.dialog.downgrade();
        let input = finder.bridge.sender().clone();
        settle();
        assert!(dialog.upgrade().is_some(), "the dialog lives while held");
        drop(finder);
        settle();
        assert!(dialog.upgrade().is_none(), "the dialog is freed");
        assert!(
            input.send(FinderInput::Close).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn a_query_is_asked_for_and_the_reply_listed_without_a_preview() {
        on_gtk_thread(query_is_asked_then_listed);
    }

    /// Opening the finder only queues; when the main loop runs it asks
    /// the window what the query matches, lists the reply under a
    /// heading, verbs first, and selects the first verb without telling
    /// anyone. Enter runs it once the finder has closed.
    fn query_is_asked_then_listed() {
        let finder = Rc::new(Finder::new());
        let asked: Rc<RefCell<Vec<String>>> = Rc::default();
        let log = Rc::clone(&asked);
        let heard: Rc<RefCell<Vec<(String, Choice)>>> = Rc::default();
        let ran: Rc<RefCell<Vec<usize>>> = Rc::default();
        let (hits, actions) = (Rc::clone(&heard), Rc::clone(&ran));
        answer(
            &finder,
            move |query| {
                log.borrow_mut().push(query.to_owned());
                vec![file("doors"), file("drawers")]
            },
            |_| {
                vec![Action {
                    label: "Add Door".to_owned(),
                    detail: "in this space".to_owned(),
                }]
            },
            move |output| match output {
                FinderOutput::Action(index) => actions.borrow_mut().push(index),
                other => hits.borrow_mut().extend(hit_of(other)),
            },
            true,
        );
        let (window, parent) = window();
        let list: gtk::ListBox = find(&finder);

        finder.present(&parent, "d");
        assert!(asked.borrow().is_empty(), "opening is only queued");
        assert!(list.row_at_index(0).is_none(), "nothing listed yet");

        settle();
        assert_eq!(*asked.borrow(), ["d"], "asked once");
        // DO, Add Door, FILES, doors, drawers.
        assert!(list.row_at_index(4).is_some() && list.row_at_index(5).is_none());
        assert_eq!(
            list.selected_row().map(|row| row.index()),
            Some(1),
            "the verb is selected"
        );
        assert!(heard.borrow().is_empty(), "filling previews nothing");

        // ↓ rests on doors, which previews it.
        let entry: gtk::SearchEntry = find(&finder);
        finder.bridge.send(FinderInput::Step(Direction::Down));
        settle();
        assert_eq!(*heard.borrow(), [("doors".to_owned(), Choice::Preview)]);
        // ↑ and Enter run the verb.
        finder.bridge.send(FinderInput::Step(Direction::Up));
        entry.emit_activate();
        settle();
        assert_eq!(*ran.borrow(), [0]);
        assert_eq!(heard.borrow().len(), 1, "a verb previews nothing");

        finder.dialog.force_close();
        window.destroy();
        settle();
    }

    #[test]
    fn with_nobody_listening_for_hits_a_hit_leaves_the_finder_open() {
        on_gtk_thread(unheard_hit_stays_open);
    }

    /// As before the finder was a component: Enter on a hit with no hit
    /// callback connected does nothing, and the finder stays open. Once a
    /// callback is connected, Enter closes it and goes there.
    fn unheard_hit_stays_open() {
        let finder = Rc::new(Finder::new());
        answer(
            &finder,
            |_| vec![file("doors")],
            |_| Vec::new(),
            |_| {},
            false,
        );
        let closed = Rc::new(Cell::new(0_u32));
        let count = Rc::clone(&closed);
        finder
            .dialog
            .connect_closed(move |_| count.set(count.get() + 1));
        let (window, parent) = window();
        finder.present(&parent, "d");
        settle();
        let list: gtk::ListBox = find(&finder);
        let entry: gtk::SearchEntry = find(&finder);
        // FILES, doors: Enter on doors.
        let Some(doors) = list.row_at_index(1) else {
            panic!("doors is listed");
        };
        list.select_row(Some(&doors));
        entry.emit_activate();
        settle();
        assert_eq!(closed.get(), 0, "the finder stays open");

        let heard: Rc<RefCell<Vec<(String, Choice)>>> = Rc::default();
        let log = Rc::clone(&heard);
        answer(
            &finder,
            |_| vec![file("doors")],
            |_| Vec::new(),
            move |output| log.borrow_mut().extend(hit_of(output)),
            true,
        );
        entry.emit_activate();
        settle();
        assert_eq!(*heard.borrow(), [("doors".to_owned(), Choice::Go)]);
        assert_eq!(closed.get(), 1, "and now Enter closes it");

        window.destroy();
        settle();
    }

    #[test]
    fn a_signal_fired_while_handling_a_choice_is_queued_not_re_entered() {
        on_gtk_thread(choices_are_queued);
    }

    /// The old finder called the window from inside GTK's signal, with
    /// its own state borrowed, and a callback that moved the selection
    /// re-entered it. Now a selection only queues a message; the choice
    /// reaches the callback after the update has returned (reading the
    /// model there would panic if it had not), and Enter pressed from
    /// inside the callback is queued too.
    fn choices_are_queued() {
        let finder = Rc::new(Finder::new());
        answer(
            &finder,
            |_| vec![file("doors"), file("drawers")],
            |_| Vec::new(),
            |_| {},
            false,
        );
        let (window, parent) = window();
        finder.present(&parent, "d");
        settle();
        let list: gtk::ListBox = find(&finder);
        let entry: gtk::SearchEntry = find(&finder);

        // (what was chosen, how, how many rows the model had then, and
        // how many choices were heard once Enter had been pressed)
        type Heard = (String, Choice, usize, usize);
        let heard: Rc<RefCell<Vec<Heard>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&finder);
        let inner = entry.clone();
        let chosen = move |output| {
            let Some((label, choice)) = hit_of(output) else {
                return;
            };
            let Some(finder) = weak.upgrade() else {
                return;
            };
            let rows = finder.bridge.model().rows.len();
            let first = log.borrow().is_empty();
            if first {
                // Enter from in here is only queued.
                inner.emit_activate();
            }
            let count = log.borrow().len();
            log.borrow_mut().push((label, choice, rows, count));
        };
        answer(
            &finder,
            |_| vec![file("doors"), file("drawers")],
            |_| Vec::new(),
            chosen,
            true,
        );

        // FILES, doors, drawers: rest on drawers.
        let Some(drawers) = list.row_at_index(2) else {
            panic!("drawers is listed");
        };
        list.select_row(Some(&drawers));
        assert!(heard.borrow().is_empty(), "the selection is only queued");

        settle();
        assert_eq!(
            *heard.borrow(),
            [
                ("drawers".to_owned(), Choice::Preview, 3, 0),
                ("drawers".to_owned(), Choice::Go, 3, 1),
            ],
            "each choice after its update, Enter after the preview"
        );

        finder.dialog.force_close();
        window.destroy();
        settle();
    }
}
