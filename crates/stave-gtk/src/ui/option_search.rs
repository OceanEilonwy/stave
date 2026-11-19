//! The search over every option of a setting (`docs/editing.md`, "The
//! short list and the search"), opened by "Search…" at the foot of the
//! setting's short list on the card. A field and the options it matches,
//! grouped as the menu groups its entries (the project's own, imported
//! modules, then `std`), each with its name, the first sentence of its doc
//! comment and its fit as the card shows it. What matches, in what order,
//! comes from `stave_app::choices::search` through the window; this dialog
//! only lists it and says which option was chosen.
//!
//! Keys: typing filters; ↑ and ↓ move through the options; Enter chooses
//! (a refused option can't be chosen: it says why instead); Escape closes.
//!
//! It is part of the window, over a dimmed backdrop in the middle of it,
//! rather than an `AdwDialog`: the window's screenshot doesn't draw an
//! `AdwDialog` opened by an edit script (as the removal's dialog found),
//! and the search has to be recorded.
//!
//! It is not a relm4 component yet (docs/relm4-migration.md): the edit
//! view, and its search test, rely on [`OptionSearch::present`],
//! [`OptionSearch::choose_selected`] and the arrow keys taking effect at
//! once, which queued messages would break. It moves when the edit view
//! does. Until then its state is in cells, shared out and never borrowed
//! across a GTK call or a callback.

use stave_app::i18n::{fill_in, ntr, tr};
use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::choices::{SearchGroup, SearchHit};
use stave_app::edit::OptionFit;

use super::edit_card::fit_words;
use super::editor::current;

type SearchCallback = Rc<dyn Fn(&str) -> Vec<SearchGroup>>;
type ChosenCallback = Rc<dyn Fn(String, String)>;

/// The search dialog for one setting at a time.
pub struct OptionSearch {
    /// The backdrop over the whole window, the dialog in its middle;
    /// hidden while closed.
    pub root: gtk::Box,
    title: gtk::Label,
    entry: gtk::SearchEntry,
    list: gtk::ListBox,
    count: gtk::Label,
    /// Each row that is an option, with its hit: shared out of the cell
    /// (see [`current`]), never borrowed while GTK is called.
    rows: Cell<Rc<[OptionRow]>>,
    /// The setting it searches, while open.
    setting: Cell<Option<String>>,
    on_search: Cell<Option<SearchCallback>>,
    on_chosen: Cell<Option<ChosenCallback>>,
    on_closed: Cell<Option<Rc<dyn Fn()>>>,
}

/// A row that is an option, and the option.
type OptionRow = (gtk::ListBoxRow, SearchHit);

impl OptionSearch {
    /// A closed dialog.
    pub fn new() -> Rc<OptionSearch> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text(tr("Search by name or by what it is"))
            .hexpand(true)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label(&tr("Search the options"))]);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["stave-option-search-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&tr("Options that match"))]);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .min_content_height(160)
            .max_content_height(460)
            .child(&list)
            .build();
        let count = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["stave-option-search-count"])
            .build();
        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-option-search-foot"])
            .build();
        foot.append(&count);
        for words in [tr("↑ ↓ move"), tr("Enter choose"), tr("Esc close")] {
            foot.append(&gtk::Label::new(Some(&words)));
        }
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-option-search"])
            .build();
        content.append(&entry);
        content.append(&scrolled);
        content.append(&foot);
        let title = gtk::Label::builder()
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["stave-option-search-title"])
            .build();
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .css_classes(["flat", "circular"])
            .tooltip_text(tr("Close (Escape)"))
            .build();
        close.update_property(&[gtk::accessible::Property::Label(&tr("Close"))]);
        let head = gtk::Box::builder()
            .css_classes(["stave-option-search-head"])
            .build();
        head.append(&title);
        head.append(&close);
        content.prepend(&head);
        content.set_width_request(560);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        content.set_hexpand(true);
        content.set_vexpand(true);
        content.add_css_class("stave-card-panel");
        content.set_accessible_role(gtk::AccessibleRole::Dialog);
        let root = gtk::Box::builder()
            .css_classes(["stave-option-search-backdrop"])
            .hexpand(true)
            .vexpand(true)
            .visible(false)
            .build();
        root.append(&content);
        let search = Rc::new(OptionSearch {
            root,
            title: title.clone(),
            entry,
            list,
            count,
            rows: Cell::new(Rc::from([])),
            setting: Cell::new(None),
            on_search: Cell::new(None),
            on_chosen: Cell::new(None),
            on_closed: Cell::new(None),
        });
        search.connect();
        let weak = Rc::downgrade(&search);
        close.connect_clicked(move |_| {
            if let Some(search) = weak.upgrade() {
                search.close();
            }
        });
        search
    }

    /// Closes it (choosing nothing, unless an option was chosen just
    /// before).
    pub fn close(&self) {
        self.root.set_visible(false);
        self.setting.set(None);
        let callback = current(&self.on_closed);
        if let Some(callback) = callback {
            callback();
        }
    }

    /// Calls `callback` each time it closes, to give the keyboard back.
    pub fn connect_closed(&self, callback: impl Fn() + 'static) {
        self.on_closed.set(Some(Rc::new(callback)));
    }

    /// Calls `callback` with the query to list the options it matches.
    pub fn connect_search(&self, callback: impl Fn(&str) -> Vec<SearchGroup> + 'static) {
        self.on_search.set(Some(Rc::new(callback)));
    }

    /// Calls `callback` with the setting and the option chosen (its text),
    /// once the dialog has closed.
    pub fn connect_chosen(&self, callback: impl Fn(String, String) + 'static) {
        self.on_chosen.set(Some(Rc::new(callback)));
    }

    /// Opens the search for a setting, with `query` typed.
    pub fn present(self: &Rc<Self>, setting: &str, query: &str) {
        self.setting.set(Some(setting.to_owned()));
        let words = fill_in(
            &tr("Choose {setting}"),
            &[("setting", &setting.replace('_', " "))],
        );
        self.title.set_label(&words);
        self.root
            .first_child()
            .inspect(|dialog| dialog.update_property(&[gtk::accessible::Property::Label(&words)]));
        self.entry.set_text(query);
        self.refresh();
        self.root.set_visible(true);
        self.entry.grab_focus();
        self.entry.set_position(-1);
    }

    /// The setting it searches, while it is open.
    pub fn setting(&self) -> Option<String> {
        current(&self.setting)
    }

    fn connect(self: &Rc<Self>) {
        let search = Rc::downgrade(self);
        self.entry.connect_search_changed(move |_| {
            if let Some(search) = search.upgrade() {
                search.refresh();
            }
        });
        let search = Rc::downgrade(self);
        self.entry.connect_activate(move |_| {
            if let Some(search) = search.upgrade() {
                search.choose_selected();
            }
        });
        let search = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            if let Some(search) = search.upgrade() {
                search.list.select_row(Some(row));
                search.choose_selected();
            }
        });
        // Up and down in the field move through the list; Escape closes
        // the dialog (the search field would only clear its text).
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let search = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(search) = search.upgrade() else {
                return gtk::glib::Propagation::Proceed;
            };
            match key {
                gtk::gdk::Key::Down => search.step(1),
                gtk::gdk::Key::Up => search.step(-1),
                gtk::gdk::Key::Escape => {
                    search.close();
                }
                _ => return gtk::glib::Propagation::Proceed,
            }
            gtk::glib::Propagation::Stop
        });
        self.entry.add_controller(keys);
    }

    /// Lists what the query matches again (and after each option's fit is
    /// found), keeping the selection on the option it was on.
    pub fn refresh(&self) {
        let query = self.entry.text().to_string();
        let callback = current(&self.on_search);
        let groups = callback.map(|search| search(&query)).unwrap_or_default();
        let selected = self.selected_hit().map(|hit| hit.option.text);
        self.fill(&groups, &query);
        let rows = current(&self.rows);
        let again = selected
            .and_then(|text| rows.iter().find(|(_, hit)| hit.option.text == text))
            .or_else(|| rows.iter().find(|(_, hit)| hit.choosable()))
            .or_else(|| rows.first());
        self.list.select_row(again.map(|(row, _)| row));
    }

    fn fill(&self, groups: &[SearchGroup], query: &str) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let mut rows = Vec::new();
        let mut found = 0;
        let mut fitting = 0;
        for group in groups {
            let heading = gtk::ListBoxRow::builder()
                .selectable(false)
                .activatable(false)
                .focusable(false)
                .child(
                    &gtk::Label::builder()
                        .label(group.title().to_uppercase())
                        .xalign(0.0)
                        .css_classes(["stave-option-search-group"])
                        .build(),
                )
                .build();
            self.list.append(&heading);
            for hit in &group.hits {
                found += 1;
                if hit.option.fit == OptionFit::Fits {
                    fitting += 1;
                }
                let row = hit_row(hit);
                self.list.append(&row);
                rows.push((row, hit.clone()));
            }
        }
        if rows.is_empty() {
            let words = if query.trim().is_empty() {
                tr("This setting has no options to choose from; type a value on the card.")
            } else {
                fill_in(
                    &tr("Nothing is called “{query}”, and nothing says so in its description."),
                    &[("query", &query)],
                )
            };
            self.list.append(
                &gtk::ListBoxRow::builder()
                    .selectable(false)
                    .activatable(false)
                    .child(
                        &gtk::Label::builder()
                            .label(words)
                            .wrap(true)
                            .xalign(0.0)
                            .css_classes(["stave-option-search-hint"])
                            .margin_start(12)
                            .margin_end(12)
                            .margin_top(12)
                            .margin_bottom(12)
                            .build(),
                    )
                    .build(),
            );
        }
        self.count.set_label(&fill_in(
            &tr("{options} · {fitting}"),
            &[
                (
                    "options",
                    &fill_in(
                        &ntr("{count} option", "{count} options", found),
                        &[("count", &found)],
                    ),
                ),
                (
                    "fitting",
                    &fill_in(
                        &ntr("{count} fits", "{count} fit", fitting),
                        &[("count", &fitting)],
                    ),
                ),
            ],
        ));
        self.rows.set(Rc::from(rows));
    }

    /// Moves the selection by `by` options.
    fn step(&self, by: i32) {
        let rows = current(&self.rows);
        if rows.is_empty() {
            return;
        }
        let at = self
            .list
            .selected_row()
            .and_then(|selected| rows.iter().position(|(row, _)| *row == selected));
        let last = rows.len().saturating_sub(1);
        let next = match (at, by.is_negative()) {
            (None, _) => 0,
            (Some(at), true) => at.saturating_sub(1),
            (Some(at), false) => at.saturating_add(1).min(last),
        };
        if let Some((row, _)) = rows.get(next) {
            self.list.select_row(Some(row));
            // Focusing the row scrolls it into sight; the keyboard goes
            // back to the field, its text not selected, to go on typing.
            row.grab_focus();
            self.entry.grab_focus();
            self.entry.set_position(-1);
        }
    }

    /// The hit the selection is on.
    fn selected_hit(&self) -> Option<SearchHit> {
        let selected = self.list.selected_row()?;
        current(&self.rows)
            .iter()
            .find(|(row, _)| *row == selected)
            .map(|(_, hit)| hit.clone())
    }

    /// Enter, or a click: the selected option, unless it was refused (then
    /// the dialog stays open, and the row says why).
    pub fn choose_selected(&self) {
        let Some(hit) = self.selected_hit() else {
            return;
        };
        if !hit.choosable() {
            self.count.set_label(&fill_in(
                &tr("{option} can't be used here: choose one that fits"),
                &[("option", &hit.option.text)],
            ));
            return;
        }
        let Some(setting) = self.setting() else {
            return;
        };
        self.close();
        let callback = current(&self.on_chosen);
        if let Some(callback) = callback {
            callback(setting, hit.option.text);
        }
    }

    /// Types `query` in the field, as typing does (for tests).
    #[cfg(test)]
    pub fn type_query(&self, query: &str) {
        self.entry.set_text(query);
        self.refresh();
    }

    /// Each option row with its text (for tests).
    #[cfg(test)]
    pub fn rows(&self) -> Vec<(gtk::ListBoxRow, String)> {
        current(&self.rows)
            .iter()
            .map(|(row, hit)| (row.clone(), hit.option.text.clone()))
            .collect()
    }

    /// The text of the option selected (for tests).
    #[cfg(test)]
    pub fn selected(&self) -> Option<String> {
        self.selected_hit().map(|hit| hit.option.text)
    }

    /// Moves the selection as ↑ and ↓ do (for tests).
    #[cfg(test)]
    pub fn press_arrow(&self, down: bool) {
        self.step(if down { 1 } else { -1 });
    }
}

/// One option: its name, how it matched, its fit in an icon and words, and
/// its description; a refused one greyed, with the checker's reason.
fn hit_row(hit: &SearchHit) -> gtk::ListBoxRow {
    let fit = &hit.option.fit;
    let content = gtk::Box::new(gtk::Orientation::Vertical, 1);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::builder()
        .label(&hit.option.text)
        .xalign(0.0)
        .css_classes(["stave-option-search-name"])
        .build();
    line.append(&name);
    let (class, short) = match fit {
        OptionFit::NotChecked => ("checking", tr("Checking whether it fits…")),
        OptionFit::Fits => ("fits", tr("Fits")),
        OptionFit::NeedsChoice(_) => ("needs", fit_words(fit)),
        OptionFit::Refused(_) => ("refused", tr("Can't be used here")),
    };
    let state = gtk::Box::builder()
        .spacing(4)
        .hexpand(true)
        .halign(gtk::Align::End)
        .css_classes(["stave-option-search-fit", class])
        .build();
    let mark: gtk::Widget = match fit {
        OptionFit::NotChecked => gtk::Spinner::builder().spinning(true).build().upcast(),
        OptionFit::Fits | OptionFit::NeedsChoice(_) | OptionFit::Refused(_) => {
            gtk::Image::from_icon_name(super::edit_card::fit_icon(fit)).upcast()
        }
    };
    state.append(&mark);
    state.append(&gtk::Label::new(Some(&short)));
    line.append(&state);
    content.append(&line);
    if let Some(hint) = &hit.option.hint {
        content.append(
            &gtk::Label::builder()
                .label(hint)
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .css_classes(["stave-option-search-hint"])
                .build(),
        );
    }
    if let OptionFit::Refused(refusal) = fit {
        content.append(
            &gtk::Label::builder()
                .label(refusal.line())
                .xalign(0.0)
                .wrap(true)
                .wrap_mode(gtk::pango::WrapMode::WordChar)
                .css_classes(["stave-option-search-fit", "refused"])
                .build(),
        );
    }
    let row = gtk::ListBoxRow::builder()
        .child(&content)
        .css_classes(["stave-option-search-hit"])
        .build();
    let description = [Some(fit_words(fit)), hit.option.hint.clone()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(". ");
    row.update_property(&[
        gtk::accessible::Property::Label(&hit.option.text),
        gtk::accessible::Property::Description(&description),
    ]);
    if !hit.choosable() {
        // Still selectable, so its reason can be read, but it can't be
        // chosen and says so.
        row.add_css_class("refused");
        row.update_state(&[gtk::accessible::State::Disabled(true)]);
    }
    row
}
