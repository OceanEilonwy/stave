//! The search over every option of a setting (`docs/editing.md`, "The
//! short list and the search"), opened by "Search…" at the foot of the
//! setting's short list on the card. A field and the options it matches,
//! grouped as the menu groups its entries (the project's own, imported
//! modules, then `std`), each with its name, the first sentence of its doc
//! comment and its fit as the card shows it. What matches, in what order,
//! comes from `stave_app::choices::search`; this dialog only lists it and
//! says which option was chosen.
//!
//! Keys: typing filters; ↑ and ↓ move through the options; Enter chooses
//! (a refused option can't be chosen: it says why instead); Escape closes.
//!
//! It is part of the window, over a dimmed backdrop in the middle of it,
//! rather than an `AdwDialog`: the window's screenshot doesn't draw an
//! `AdwDialog` opened by an edit script (as the removal's dialog found),
//! and the search has to be recorded.
//!
//! It is a relm4 component, [`OptionSearchComponent`], owned by the
//! window (docs/relm4-migration.md). It never asks the window anything:
//! the window gives it a copy of the setting it searches when it opens it
//! and again as each option's fit is found
//! ([`OptionSearchInput::Update`]), and it searches that copy itself. Its
//! widgets only send it messages; it tells the window when it closes and
//! what was chosen.

use stave_app::i18n::{fill_in, ntr, tr};

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::choices::{SearchGroup, SearchHit};
use stave_app::edit::{OptionFit, SettingParam};

use super::edit_card::fit_words;

/// A row that is an option, and the option.
type OptionRow = (gtk::ListBoxRow, SearchHit);

/// The search dialog for one setting at a time, as a relm4 component. Its
/// model is what it searches and what it lists; only
/// [`OptionSearchComponent::update_with_view`] changes it.
#[derive(Debug, Default)]
pub struct OptionSearchComponent {
    /// The setting it searches, by name, while open.
    open: Option<String>,
    /// The window's copy of that setting (none when the card has no such
    /// setting: then nothing matches).
    setting: Option<SettingParam>,
    /// Each row that is an option, with its hit, in order.
    rows: Vec<OptionRow>,
}

/// What the search is told.
#[derive(Debug)]
pub enum OptionSearchInput {
    /// Open the search over this setting, with `query` typed.
    Present {
        /// The setting's name.
        name: String,
        /// The setting as the session has it now, to search.
        setting: Option<Box<SettingParam>>,
        /// What is typed in the field.
        query: String,
    },
    /// The setting as the session has it now (an option's fit was found):
    /// listed again if the search is open on it, the selection kept.
    Update(Box<SettingParam>),
    /// Close it, choosing nothing (Escape, or its close button).
    Close,
    /// What is typed in the field changed.
    QueryChanged,
    /// Choose the selected option (Enter).
    ChooseSelected,
    /// A row was activated (a click): choose it.
    Activated(i32),
    /// Move the selection by this many options (↑ and ↓).
    Step(i32),
}

/// What the search tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionSearchOutput {
    /// It closed (after a choice too, before [`OptionSearchOutput::Chosen`]).
    Closed,
    /// People chose this option of this setting.
    Chosen {
        /// The setting's name.
        setting: String,
        /// The option's text.
        value: String,
    },
}

/// The search's widgets.
#[derive(Debug)]
pub struct OptionSearchWidgets {
    title: gtk::Label,
    entry: gtk::SearchEntry,
    list: gtk::ListBox,
    count: gtk::Label,
}

impl Component for OptionSearchComponent {
    type Init = ();
    type Input = OptionSearchInput;
    type Output = OptionSearchOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = OptionSearchWidgets;

    /// The backdrop over the whole window, the dialog in its middle;
    /// hidden while closed.
    fn init_root() -> gtk::Box {
        gtk::Box::builder()
            .css_classes(["stave-option-search-backdrop"])
            .hexpand(true)
            .vexpand(true)
            .visible(false)
            .build()
    }

    fn init(_: (), root: gtk::Box, sender: ComponentSender<Self>) -> ComponentParts<Self> {
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
        root.append(&content);

        let input = sender.input_sender();
        {
            let input = input.clone();
            close.connect_clicked(move |_| input.emit(OptionSearchInput::Close));
        }
        {
            let input = input.clone();
            entry.connect_search_changed(move |_| input.emit(OptionSearchInput::QueryChanged));
        }
        {
            let input = input.clone();
            entry.connect_activate(move |_| input.emit(OptionSearchInput::ChooseSelected));
        }
        {
            let input = input.clone();
            list.connect_row_activated(move |_, row| {
                input.emit(OptionSearchInput::Activated(row.index()));
            });
        }
        // Up and down in the field move through the list; Escape closes
        // the dialog (the search field would only clear its text). The
        // key is taken at once; what it does is a message.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let input = input.clone();
            keys.connect_key_pressed(move |_, key, _, _| {
                let message = match key {
                    gtk::gdk::Key::Down => OptionSearchInput::Step(1),
                    gtk::gdk::Key::Up => OptionSearchInput::Step(-1),
                    gtk::gdk::Key::Escape => OptionSearchInput::Close,
                    _ => return gtk::glib::Propagation::Proceed,
                };
                input.emit(message);
                gtk::glib::Propagation::Stop
            });
        }
        entry.add_controller(keys);
        ComponentParts {
            model: OptionSearchComponent::default(),
            widgets: OptionSearchWidgets {
                title,
                entry,
                list,
                count,
            },
        }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut OptionSearchWidgets,
        message: OptionSearchInput,
        sender: ComponentSender<Self>,
        root: &gtk::Box,
    ) {
        match message {
            OptionSearchInput::Present {
                name,
                setting,
                query,
            } => {
                let words = fill_in(
                    &tr("Choose {setting}"),
                    &[("setting", &name.replace('_', " "))],
                );
                widgets.title.set_label(&words);
                root.first_child().inspect(|dialog| {
                    dialog.update_property(&[gtk::accessible::Property::Label(&words)]);
                });
                self.open = Some(name);
                self.setting = setting.map(|setting| *setting);
                // Its search-changed handler lists again, as this does.
                widgets.entry.set_text(&query);
                self.refresh(widgets);
                root.set_visible(true);
                widgets.entry.grab_focus();
                widgets.entry.set_position(-1);
            }
            OptionSearchInput::Update(setting) => {
                if self.open.as_deref() == Some(setting.name.as_str()) {
                    self.setting = Some(*setting);
                    self.refresh(widgets);
                }
            }
            OptionSearchInput::Close => self.close(root, &sender),
            OptionSearchInput::QueryChanged => {
                if self.open.is_some() {
                    self.refresh(widgets);
                }
            }
            OptionSearchInput::ChooseSelected => self.choose_selected(widgets, root, &sender),
            OptionSearchInput::Activated(index) => {
                if let Some(row) = widgets.list.row_at_index(index) {
                    widgets.list.select_row(Some(&row));
                    self.choose_selected(widgets, root, &sender);
                }
            }
            OptionSearchInput::Step(by) => self.step(widgets, by),
        }
    }
}

impl OptionSearchComponent {
    /// Closes it, choosing nothing (unless an option was chosen just
    /// before), and says so.
    fn close(&mut self, root: &gtk::Box, sender: &ComponentSender<Self>) {
        if self.open.take().is_none() {
            return;
        }
        root.set_visible(false);
        // Nobody listening is not a mistake.
        let _ = sender.output(OptionSearchOutput::Closed);
    }

    /// Lists what the query matches again (and after each option's fit is
    /// found), keeping the selection on the option it was on.
    fn refresh(&mut self, widgets: &OptionSearchWidgets) {
        let query = widgets.entry.text().to_string();
        let groups = self
            .setting
            .as_ref()
            .map(|setting| stave_app::choices::search(setting, &query))
            .unwrap_or_default();
        let selected = self
            .selected_hit(widgets)
            .map(|hit| hit.option.text.clone());
        self.rows = fill(widgets, &groups, &query);
        let again = selected
            .and_then(|text| self.rows.iter().find(|(_, hit)| hit.option.text == text))
            .or_else(|| self.rows.iter().find(|(_, hit)| hit.choosable()))
            .or_else(|| self.rows.first());
        widgets.list.select_row(again.map(|(row, _)| row));
    }

    /// Moves the selection by `by` options.
    fn step(&self, widgets: &OptionSearchWidgets, by: i32) {
        if self.rows.is_empty() {
            return;
        }
        let at = widgets
            .list
            .selected_row()
            .and_then(|selected| self.rows.iter().position(|(row, _)| *row == selected));
        let last = self.rows.len().saturating_sub(1);
        let next = match (at, by.is_negative()) {
            (None, _) => 0,
            (Some(at), true) => at.saturating_sub(1),
            (Some(at), false) => at.saturating_add(1).min(last),
        };
        if let Some((row, _)) = self.rows.get(next) {
            widgets.list.select_row(Some(row));
            // Focusing the row scrolls it into sight; the keyboard goes
            // back to the field, its text not selected, to go on typing.
            row.grab_focus();
            widgets.entry.grab_focus();
            widgets.entry.set_position(-1);
        }
    }

    /// The hit the selection is on.
    fn selected_hit(&self, widgets: &OptionSearchWidgets) -> Option<&SearchHit> {
        let selected = widgets.list.selected_row()?;
        self.rows
            .iter()
            .find(|(row, _)| *row == selected)
            .map(|(_, hit)| hit)
    }

    /// Enter, or a click: the selected option, unless it was refused (then
    /// the dialog stays open, and the row says why).
    fn choose_selected(
        &mut self,
        widgets: &OptionSearchWidgets,
        root: &gtk::Box,
        sender: &ComponentSender<Self>,
    ) {
        let Some(hit) = self.selected_hit(widgets) else {
            return;
        };
        let value = hit.option.text.clone();
        if !hit.choosable() {
            widgets.count.set_label(&fill_in(
                &tr("{option} can't be used here: choose one that fits"),
                &[("option", &value)],
            ));
            return;
        }
        let Some(setting) = self.open.clone() else {
            return;
        };
        self.close(root, sender);
        let _ = sender.output(OptionSearchOutput::Chosen { setting, value });
    }

    /// The setting it searches, while open (for tests).
    #[cfg(test)]
    pub fn setting(&self) -> Option<&str> {
        self.open.as_deref()
    }

    /// Each option row with its text (for tests).
    #[cfg(test)]
    pub fn rows(&self) -> Vec<(gtk::ListBoxRow, String)> {
        self.rows
            .iter()
            .map(|(row, hit)| (row.clone(), hit.option.text.clone()))
            .collect()
    }
}

#[cfg(test)]
impl OptionSearchWidgets {
    /// The search field, to type in (for tests).
    pub fn entry(&self) -> gtk::SearchEntry {
        self.entry.clone()
    }

    /// The text of the option selected (for tests).
    pub fn selected(&self, rows: &[(gtk::ListBoxRow, String)]) -> Option<String> {
        let selected = self.list.selected_row()?;
        rows.iter()
            .find(|(row, _)| *row == selected)
            .map(|(_, text)| text.clone())
    }
}

/// Fills the list with `groups`, each under its heading, or says why
/// nothing matches, and counts what it lists. Returns the option rows.
fn fill(widgets: &OptionSearchWidgets, groups: &[SearchGroup], query: &str) -> Vec<OptionRow> {
    let list = &widgets.list;
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
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
                    .label(group.title())
                    .xalign(0.0)
                    .css_classes(["stave-option-search-group", "stave-caps"])
                    .build(),
            )
            .build();
        list.append(&heading);
        for hit in &group.hits {
            found += 1;
            if hit.option.fit == OptionFit::Fits {
                fitting += 1;
            }
            let row = hit_row(hit);
            list.append(&row);
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
        list.append(
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
    widgets.count.set_label(&fill_in(
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
    rows
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

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};
    use crate::ui::relm4_shim::Bridge;

    #[test]
    fn a_dropped_option_search_is_freed() {
        on_gtk_thread(option_search_is_freed);
    }

    /// Dropping the bridge shuts the search down when the main loop next
    /// runs: its backdrop, its rows and the callback are freed.
    fn option_search_is_freed() {
        let search = Bridge::<OptionSearchComponent>::launch(());
        let token = Rc::new(());
        let held = Rc::clone(&token);
        search.connect(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        search.send(OptionSearchInput::Present {
            name: "joinery".to_owned(),
            setting: None,
            query: "x".to_owned(),
        });
        settle();
        assert_eq!(search.model().setting(), Some("joinery"));
        let root = search.widget().downgrade();
        let input = search.sender().clone();
        drop(search);
        settle();
        assert!(root.upgrade().is_none(), "the backdrop is freed");
        assert!(
            input.send(OptionSearchInput::Close).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn closing_a_closed_search_says_nothing() {
        on_gtk_thread(closing_twice);
    }

    /// Escape pressed twice (the second queued before the first is
    /// handled) closes it once.
    fn closing_twice() {
        let search = Bridge::<OptionSearchComponent>::launch(());
        let heard: Rc<std::cell::RefCell<Vec<OptionSearchOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        search.connect(move |output| log.borrow_mut().push(output));
        search.send(OptionSearchInput::Present {
            name: "joinery".to_owned(),
            setting: None,
            query: String::new(),
        });
        search.send(OptionSearchInput::Close);
        search.send(OptionSearchInput::Close);
        assert!(heard.borrow().is_empty(), "only queued");
        settle();
        assert_eq!(*heard.borrow(), [OptionSearchOutput::Closed]);
        assert!(!search.widget().is_visible());
    }
}
