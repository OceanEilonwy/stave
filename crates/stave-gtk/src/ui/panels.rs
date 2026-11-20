//! Two of the docked window's panels that are lists: Problems (every
//! problem the latest check found, in full, as `stave check` prints it:
//! the heading, each place with its line of source and the part of it
//! underlined, the notes and the help line; choosing one selects it, so
//! the 3D view poses it, and each place opens in the Code panel) and Files
//! (the project's `.stave` files; choosing one opens it in the Code
//! panel).
//!
//! Each is a relm4 component ([`ProblemsComponent`], [`FilesListComponent`]):
//! what the list shows is its model, only its update changes that, and the
//! rows and links only send input messages. The rows are plain widgets the
//! update builds, not components of their own. [`ProblemsPanel`] and
//! [`FilesList`] are their owner's handles: the one owner of the
//! component's controller, its outputs forwarded at launch to where the
//! owner said, its setters sent as input messages.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::i18n::{fill_in, tr};
use stave_app::inspect::Token;
use stave_app::problems::{Report, ReportPlace, Role};
use stave_app::session::SourceLocation;
use stave_lang::Severity;

use super::style::CODE;

/// The Problems panel's state: the problems it lists, whether the 3D
/// view shows an older design than theirs, and which one is selected.
#[derive(Debug)]
pub struct ProblemsComponent {
    /// The problems the rows show, in order, so a new check replaces only
    /// the rows that changed.
    shown: Vec<Report>,
    stale: bool,
    selected: Option<usize>,
}

/// What the Problems panel is told.
#[derive(Debug)]
pub enum ProblemsInput {
    /// List these problems in full, the `selected` one marked.
    Show {
        /// The latest check's problems.
        reports: Vec<Report>,
        /// The selected one's index.
        selected: Option<usize>,
        /// The 3D view shows the last design that had no errors.
        stale: bool,
    },
    /// Mark the selected problem, or none, without telling anyone.
    ShowSelected(Option<usize>),
    /// People chose the problem in this row. A row from an older listing
    /// (replaced by a [`ProblemsInput::Show`] queued before the choice)
    /// no longer means any of the problems listed, so it is let go.
    Chose(gtk::ListBoxRow),
    /// People cleared the selection (with Ctrl and a click): nothing is
    /// selected now, and nobody is told.
    Cleared,
    /// People clicked one of a problem's places.
    OpenPlace(SourceLocation),
}

/// What the Problems panel tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemsOutput {
    /// A problem was chosen (its index in the latest check's problems).
    Choose(usize),
    /// One of a problem's places was clicked, to open in the Code panel.
    Source(SourceLocation),
}

impl ProblemsComponent {
    /// The selected problem's row, if it has one.
    fn selected_row(&self, list: &gtk::ListBox) -> Option<gtk::ListBoxRow> {
        self.selected
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| list.row_at_index(index))
    }

    /// Which page shows: the list, or why it is empty.
    fn page(&self) -> &'static str {
        if self.shown.is_empty() {
            "empty"
        } else {
            "list"
        }
    }
}

#[relm4::component(pub)]
impl Component for ProblemsComponent {
    type Init = ();
    type Input = ProblemsInput;
    type Output = ProblemsOutput;
    type CommandOutput = ();

    view! {
        gtk::Stack {
            add_named[Some("list")] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_vexpand: true,
                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    // Over the list while the 3D view shows an older design
                    // than the one these problems are about.
                    gtk::Label {
                        set_label: &tr("The 3D view shows the last version without errors until these are fixed."),
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "dim",
                        add_css_class: "caption",
                        add_css_class: "stave-problems-stale",
                        #[watch]
                        set_visible: model.stale && !model.shown.is_empty(),
                    },
                    #[name = "list"]
                    gtk::ListBox {
                        set_selection_mode: gtk::SelectionMode::Single,
                        add_css_class: "navigation-sidebar",
                        add_css_class: "stave-panel-list",
                        // The window marks the selection without hearing back.
                        #[watch]
                        #[block_signal(row_selected)]
                        select_row: model.selected_row(list.upcast_ref()).as_ref(),
                        connect_row_selected[sender] => move |_, row| {
                            sender.input_sender().emit(match row {
                                Some(row) => ProblemsInput::Chose(row.clone()),
                                None => ProblemsInput::Cleared,
                            });
                        } @row_selected,
                    },
                },
            },
            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some("object-select-symbolic"),
                set_title: &tr("No problems"),
                set_description: Some(&tr("Every part fits, every joint closes and everything that moves moves clear.")),
                add_css_class: "compact",
            },
            #[watch]
            set_visible_child_name: model.page(),
        }
    }

    fn init((): (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = ProblemsComponent {
            shown: Vec::new(),
            stale: false,
            selected: None,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: ProblemsInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ProblemsInput::Show {
                reports,
                selected,
                stale,
            } => {
                let list = &widgets.list;
                list.block_signal(&widgets.row_selected);
                replace_changed_rows(list, &self.shown, &reports, sender.input_sender());
                list.unblock_signal(&widgets.row_selected);
                self.shown = reports;
                self.stale = stale;
                self.selected = selected;
            }
            ProblemsInput::ShowSelected(selected) => self.selected = selected,
            ProblemsInput::Chose(row) => {
                let listed = row.parent().as_ref() == Some(widgets.list.upcast_ref());
                if let Some(index) = listed.then(|| usize::try_from(row.index()).ok()).flatten() {
                    self.selected = Some(index);
                    // Nobody listening is not a mistake.
                    let _ = sender.output(ProblemsOutput::Choose(index));
                }
            }
            ProblemsInput::Cleared => self.selected = None,
            ProblemsInput::OpenPlace(location) => {
                let _ = sender.output(ProblemsOutput::Source(location));
            }
        }
        self.update_view(widgets, sender);
    }
}

/// The Problems panel: it owns the [`ProblemsComponent`]'s controller
/// (dropping it shuts the component down) and passes its setters on as
/// [`ProblemsInput`] messages; its [`ProblemsOutput`]s go where the owner
/// said when it made it.
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned.
pub struct ProblemsPanel {
    /// The panel's content.
    pub root: gtk::Stack,
    controller: Controller<ProblemsComponent>,
}

impl ProblemsPanel {
    /// An empty list, its outputs (a problem chosen, by its index in the
    /// latest check's problems, or one of a problem's places clicked, to
    /// open in the Code panel) made into the owner's messages by `map` and
    /// sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(ProblemsOutput) -> X + 'static,
    ) -> ProblemsPanel {
        let controller = ProblemsComponent::builder().launch(()).forward(to, map);
        ProblemsPanel {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Queues `input`; once the component has gone there is nobody to
    /// tell.
    fn send(&self, input: ProblemsInput) {
        let _ = self.controller.sender().send(input);
    }

    /// Lists these problems in full, the `selected` one marked. `stale`
    /// says the 3D view shows the last design that had no errors. Rows
    /// for problems that are the same at the start and the end of the
    /// list are kept; only the run between is replaced, so the scroll
    /// position and the keyboard's row survive a check that changed
    /// nothing, or little.
    pub fn show(&self, reports: &[Report], selected: Option<usize>, stale: bool) {
        self.send(ProblemsInput::Show {
            reports: reports.to_vec(),
            selected,
            stale,
        });
    }

    /// Marks the selected problem, or none.
    pub fn show_selected(&self, selected: Option<usize>) {
        self.send(ProblemsInput::ShowSelected(selected));
    }
}

/// Replaces the rows showing `shown` with rows for `reports`: rows for
/// problems that are the same at the start and the end of both lists are
/// kept, and only the run between is removed and built again.
fn replace_changed_rows(
    list: &gtk::ListBox,
    shown: &[Report],
    reports: &[Report],
    input: &relm4::Sender<ProblemsInput>,
) {
    let prefix = shown
        .iter()
        .zip(reports)
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = shown
        .iter()
        .skip(prefix)
        .rev()
        .zip(reports.iter().skip(prefix).rev())
        .take_while(|(old, new)| old == new)
        .count();
    let removed = shown.len().saturating_sub(prefix).saturating_sub(suffix);
    let at = i32::try_from(prefix).unwrap_or(i32::MAX);
    for _ in 0..removed {
        if let Some(row) = list.row_at_index(at) {
            list.remove(&row);
        }
    }
    let added = reports.len().saturating_sub(prefix).saturating_sub(suffix);
    for (offset, report) in reports.iter().skip(prefix).take(added).enumerate() {
        let position = prefix
            .checked_add(offset)
            .and_then(|position| i32::try_from(position).ok())
            .unwrap_or(-1);
        list.insert(&report_row(report, input), position);
    }
}

/// One problem as `stave check` prints it: `error[require]: message`,
/// each place with its line of source, then the notes and the help.
fn report_row(report: &Report, input: &relm4::Sender<ProblemsInput>) -> gtk::Box {
    let (icon, class) = severity_look(report.severity);
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["stave-problem-row"])
        .build();
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("stave-problem-icon");
    image.add_css_class(class);
    image.set_valign(gtk::Align::Start);
    row.append(&image);
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .hexpand(true)
        .build();
    let head = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(1)
        .build();
    head.append(
        &gtk::Label::builder()
            .label(report.heading())
            .xalign(0.0)
            .css_classes(["stave-problem-code", class])
            .build(),
    );
    head.append(
        &gtk::Label::builder()
            .label(&report.message)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .css_classes(["stave-problem-heading"])
            .build(),
    );
    text.append(&head);
    for place in &report.places {
        text.append(&place_block(place, report.severity, input));
    }
    for note in &report.notes {
        text.append(&fact(&tr("note"), note, "stave-problem-note"));
    }
    if let Some(help) = &report.help {
        text.append(&fact(&tr("help"), help, "stave-problem-help"));
    }
    row.append(&text);
    row
}

/// A place: its `file:line:column` as a link to the Code panel (with
/// what it is, `the kind's rule`), over its line of source with the
/// part the problem is about underlined.
fn place_block(
    place: &ReportPlace,
    severity: Severity,
    input: &relm4::Sender<ProblemsInput>,
) -> gtk::Box {
    let block = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    let line = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let link = super::style::as_link(
        gtk::Button::builder()
            .label(&place.shown)
            .css_classes(["flat", "stave-link", "stave-problem-place"])
            .tooltip_text(tr("Show this place in the Code panel"))
            .build(),
    );
    let input = input.clone();
    let location = place.location.clone();
    link.connect_clicked(move |_| input.emit(ProblemsInput::OpenPlace(location.clone())));
    line.append(&link);
    let also_here = tr("also here");
    let what: &str = match place.role {
        Role::Primary => &place.message,
        Role::Secondary if place.message.is_empty() => &also_here,
        Role::Secondary => &place.message,
    };
    if !what.is_empty() {
        line.append(
            &gtk::Label::builder()
                .label(what)
                .xalign(0.0)
                .wrap(true)
                .css_classes(["dim", "caption"])
                .build(),
        );
    }
    block.append(&line);
    let snippet = gtk::Label::builder()
        .use_markup(true)
        .label(snippet_markup(place, severity))
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["stave-code", "stave-problem-snippet"])
        .build();
    snippet.update_property(&[gtk::accessible::Property::Label(&fill_in(
        &tr("Line {line}: {text}"),
        &[
            ("line", &place.location.line),
            (
                "text",
                &format!("{}{}{}", place.before, place.marked, place.after),
            ),
        ],
    ))]);
    block.append(&snippet);
    block
}

/// The icon and style class for a severity.
fn severity_look(severity: Severity) -> (&'static str, &'static str) {
    match severity {
        Severity::Error => ("dialog-error-symbolic", "error"),
        Severity::Warning => ("dialog-warning-symbolic", "warning"),
        Severity::Note => ("dialog-information-symbolic", "note"),
    }
}

fn escape(text: &str) -> String {
    gtk::glib::markup_escape_text(text).to_string()
}

/// A note or the help line: `note: for side_l and top in top_dowels`.
fn fact(word: &str, text: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .use_markup(true)
        .label(format!(
            "<span weight=\"700\">{}:</span> {}",
            escape(word),
            escape(text)
        ))
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes([class])
        .build()
}

/// A place's line of source as Pango markup: the line number dim, the
/// text coloured as the editor colours it, without its indent, and the
/// part the problem is about with a wavy underline (red for an error,
/// amber for a warning; a place that explains it dotted).
fn snippet_markup(place: &ReportPlace, severity: Severity) -> String {
    let whole = format!("{}{}{}", place.before, place.marked, place.after);
    let indent = whole.chars().take_while(|c| *c == ' ').count();
    let start = place.before.chars().count();
    let end = start.saturating_add(place.marked.chars().count());
    let tokens = stave_app::inspect::highlight(&whole);
    let (before, marked, after) = split_tokens(&tokens, indent.min(start), start, end);
    let colour = match severity {
        Severity::Error => CODE.error,
        Severity::Warning | Severity::Note => CODE.warning,
    };
    let (underline, tint) = match place.role {
        Role::Primary => ("error", "24%"),
        Role::Secondary => ("single", "12%"),
    };
    format!(
        "<span foreground=\"{comment}\">{line} │</span> {before}<span underline=\"{underline}\" underline_color=\"{colour}\" background=\"{colour}\" bgalpha=\"{tint}\">{marked}</span>{after}",
        comment = CODE.comment,
        line = place.location.line,
        before = super::inspector::line_markup(&before),
        marked = super::inspector::line_markup(&marked),
        after = super::inspector::line_markup(&after),
    )
}

/// Splits highlighted tokens at character positions: those from `from`
/// to `start`, from `start` to `end`, and from `end` on (what is before
/// `from`, the indent, is left out).
fn split_tokens(
    tokens: &[Token],
    from: usize,
    start: usize,
    end: usize,
) -> (Vec<Token>, Vec<Token>, Vec<Token>) {
    let mut parts = (Vec::new(), Vec::new(), Vec::new());
    let mut at = 0usize;
    for token in tokens {
        let mut piece = String::new();
        let mut piece_part = Part::Indent;
        for character in token.text.chars() {
            let part = if at < from {
                Part::Indent
            } else if at < start {
                Part::Before
            } else if at < end {
                Part::Marked
            } else {
                Part::After
            };
            if part != piece_part && !piece.is_empty() {
                push_piece(&mut parts, piece_part, token, std::mem::take(&mut piece));
            }
            piece_part = part;
            piece.push(character);
            at = at.saturating_add(1);
        }
        if !piece.is_empty() {
            push_piece(&mut parts, piece_part, token, piece);
        }
    }
    parts
}

/// Which part of a snippet's line a character is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Indent,
    Before,
    Marked,
    After,
}

fn push_piece(
    parts: &mut (Vec<Token>, Vec<Token>, Vec<Token>),
    part: Part,
    token: &Token,
    text: String,
) {
    let piece = Token {
        class: token.class,
        text,
    };
    match part {
        Part::Indent => {}
        Part::Before => parts.0.push(piece),
        Part::Marked => parts.1.push(piece),
        Part::After => parts.2.push(piece),
    }
}

/// The Files panel's state: the `.stave` files it lists, in order.
#[derive(Debug)]
pub struct FilesListComponent {
    /// Shared, never changed, with the list's `row-activated` handler for
    /// this listing, so a row always names the file it shows.
    paths: Rc<[PathBuf]>,
}

/// What the Files panel is told.
#[derive(Debug)]
pub enum FilesListInput {
    /// List the `.stave` files in this project folder and the folders in
    /// it.
    SetFolder(PathBuf),
    /// People chose this file (the one its row showed when they chose
    /// it, even if another folder's listing is queued).
    Activated(PathBuf),
}

/// What the Files panel tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesListOutput {
    /// A file was chosen.
    Open(PathBuf),
}

impl FilesListComponent {
    /// Which page shows: the list, or why it is empty.
    fn page(&self) -> &'static str {
        if self.paths.is_empty() {
            "empty"
        } else {
            "list"
        }
    }
}

/// One file's row: its path inside the project folder.
fn file_row(path: &Path, folder: &Path) -> gtk::Box {
    let shown = path.strip_prefix(folder).unwrap_or(path);
    let label = gtk::Label::builder()
        .label(shown.display().to_string())
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .build();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
    row.append(&label);
    row
}

#[relm4::component(pub)]
impl Component for FilesListComponent {
    type Init = ();
    type Input = FilesListInput;
    type Output = FilesListOutput;
    type CommandOutput = ();

    view! {
        gtk::Stack {
            add_named[Some("list")] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_vexpand: true,
                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    #[name = "list"]
                    gtk::ListBox {
                        set_selection_mode: gtk::SelectionMode::None,
                        set_activate_on_single_click: true,
                        add_css_class: "navigation-sidebar",
                        add_css_class: "stave-panel-list",
                        // Compact rows, like the Parts panel's (style.rs, SHELL).
                        add_css_class: "stave-files-list",
                    },
                },
            },
            add_named[Some("empty")] = &adw::StatusPage {
                set_icon_name: Some("folder-symbolic"),
                set_title: &tr("No files"),
                set_description: Some(&tr("This folder has no .stave files.")),
                add_css_class: "compact",
            },
            #[watch]
            set_visible_child_name: model.page(),
        }
    }

    additional_fields! {
        /// The list's `row-activated` handler for the current listing.
        activation: Option<glib::SignalHandlerId>,
    }

    fn init((): (), root: Self::Root, _sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = FilesListComponent {
            paths: Rc::from(Vec::new()),
        };
        let activation = None;
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: FilesListInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            FilesListInput::SetFolder(folder) => {
                let mut paths = Vec::new();
                collect_stave_files(&folder, &mut paths);
                paths.sort();
                let paths: Rc<[PathBuf]> = Rc::from(paths);
                let list = &widgets.list;
                if let Some(handler) = widgets.activation.take() {
                    list.disconnect(handler);
                }
                list.remove_all();
                for path in paths.iter() {
                    list.append(&file_row(path, &folder));
                }
                // The rows and the files they show change together: a row
                // activated before this listing is handled still names its
                // own file, and one activated after names the new one.
                let shown = Rc::clone(&paths);
                let input = sender.input_sender().clone();
                widgets.activation = Some(list.connect_row_activated(move |_, row| {
                    let path = usize::try_from(row.index())
                        .ok()
                        .and_then(|index| shown.get(index));
                    if let Some(path) = path {
                        input.emit(FilesListInput::Activated(path.clone()));
                    }
                }));
                self.paths = paths;
            }
            FilesListInput::Activated(path) => {
                // Nobody listening is not a mistake.
                let _ = sender.output(FilesListOutput::Open(path));
            }
        }
        self.update_view(widgets, sender);
    }
}

/// The Files panel: it owns the [`FilesListComponent`]'s controller
/// (dropping it shuts the component down) and passes its setter on as a
/// [`FilesListInput`] message; its [`FilesListOutput`]s go where the owner
/// said when it made it.
///
/// The setter is queued: it takes effect when the main loop next runs,
/// after whatever called it has returned.
pub struct FilesList {
    /// The panel's content.
    pub root: gtk::Stack,
    controller: Controller<FilesListComponent>,
}

impl FilesList {
    /// An empty list, each file people choose made into the owner's
    /// message by `map` and sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(FilesListOutput) -> X + 'static,
    ) -> FilesList {
        let controller = FilesListComponent::builder().launch(()).forward(to, map);
        FilesList {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Lists the `.stave` files in a project folder and the folders in it
    /// (not hidden ones, nor `target`), sorted by path.
    pub fn set_folder(&self, folder: &Path) {
        let _ = self
            .controller
            .sender()
            .send(FilesListInput::SetFolder(folder.to_path_buf()));
    }
}

/// How deep the Files panel looks into folders.
const FOLDER_DEPTH: usize = 6;

fn collect_stave_files(folder: &Path, into: &mut Vec<PathBuf>) {
    let mut pending = vec![(folder.to_path_buf(), 0)];
    while let Some((current, depth)) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < FOLDER_DEPTH {
                pending.push((path, depth + 1));
            } else if kind.is_file() && path.extension().is_some_and(|e| e == "stave") {
                into.push(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use stave_app::session::LineNumber;

    use super::*;
    use crate::ui::gtk_test::{listener, on_gtk_thread, recorder, settle};

    #[test]
    fn the_cabinet_lists_its_own_files() {
        let folder = cabinet();
        let mut paths = Vec::new();
        collect_stave_files(&folder, &mut paths);
        assert!(paths.iter().any(|p| p.ends_with("doors.stave")));
        assert!(
            paths
                .iter()
                .all(|p| p.extension().is_some_and(|e| e == "stave"))
        );
    }

    /// The clothing cabinet example's folder.
    fn cabinet() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet")
    }

    /// The list inside a panel.
    fn list_in(widget: &gtk::Widget) -> Option<gtk::ListBox> {
        if let Ok(list) = widget.clone().downcast::<gtk::ListBox>() {
            return Some(list);
        }
        std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
            .find_map(|child| list_in(&child))
    }

    /// The panel's list, which every panel has.
    fn list_of(root: &gtk::Stack) -> gtk::ListBox {
        let Some(list) = list_in(root.upcast_ref()) else {
            panic!("the panel has a list");
        };
        list
    }

    /// A problem at line `line` of `doors.stave`.
    fn report(line: u32) -> Report {
        let Some(line) = LineNumber::new(line) else {
            panic!("lines count from 1");
        };
        let location = SourceLocation {
            path: "doors.stave".to_owned(),
            line,
        };
        Report {
            severity: Severity::Error,
            code: *stave_lang::Code::ALL.first().expect("a code"),
            message: "door_l hits side_l".to_owned(),
            places: vec![ReportPlace {
                role: Role::Primary,
                shown: format!("doors.stave:{line}:5"),
                location,
                before: "    ".to_owned(),
                marked: "door_l".to_owned(),
                after: " = door {}".to_owned(),
                message: String::new(),
            }],
            notes: vec!["for door_l and side_l".to_owned()],
            help: Some("open it less far".to_owned()),
        }
    }

    /// A problem saying `message`, with no places.
    fn problem(message: &str) -> Report {
        Report {
            severity: Severity::Error,
            code: *stave_lang::Code::ALL.first().expect("a code"),
            message: message.to_owned(),
            places: Vec::new(),
            notes: Vec::new(),
            help: None,
        }
    }

    /// The row widgets, in order.
    fn rows(list: &gtk::ListBox) -> Vec<gtk::ListBoxRow> {
        (0..).map_while(|index| list.row_at_index(index)).collect()
    }

    #[test]
    fn a_new_check_replaces_only_the_problems_that_changed() {
        on_gtk_thread(problems_update_in_place);
    }

    fn problems_update_in_place() {
        let (to, _heard) = recorder();
        let panel = ProblemsPanel::new(&to, |output| output);
        let list = list_of(&panel.root);
        let first = vec![problem("a"), problem("b"), problem("c")];
        panel.show(&first, None, false);
        settle();
        let before = rows(&list);
        assert_eq!(before.len(), 3);
        panel.show(&first, None, false);
        settle();
        assert_eq!(rows(&list), before, "the same problems keep their rows");
        let second = vec![problem("a"), problem("x"), problem("y"), problem("c")];
        panel.show(&second, Some(2), false);
        settle();
        let after = rows(&list);
        assert_eq!(after.len(), 4);
        assert_eq!(after.first(), before.first(), "a is kept");
        assert_eq!(after.last(), before.last(), "c is kept");
        assert!(!after.contains(&before[1]), "b is replaced");
        assert_eq!(
            list.selected_row().map(|row| row.index()),
            Some(2),
            "the selected problem is marked"
        );
        panel.show(&[], None, false);
        settle();
        assert!(rows(&list).is_empty());
    }

    #[test]
    fn a_dropped_problems_panel_is_freed() {
        on_gtk_thread(problems_panel_is_freed);
    }

    /// Dropping the panel shuts its component down when the main loop
    /// next runs: the list, the model and the callbacks are all freed.
    fn problems_panel_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let to = listener(move |_: ProblemsOutput| {
            let _ = &held;
        });
        let panel = ProblemsPanel::new(&to, |output| output);
        drop(to);
        let callbacks = Rc::downgrade(&token);
        drop(token);
        panel.show(&[report(3)], None, false);
        settle();
        let root = panel.root.downgrade();
        let input = panel.controller.sender().clone();
        drop(panel);
        settle();
        assert!(root.upgrade().is_none(), "the panel is freed");
        assert!(
            input.send(ProblemsInput::ShowSelected(None)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callbacks.upgrade().is_none(), "the callbacks are freed");
    }

    #[test]
    fn setters_mark_problems_without_calling_back() {
        on_gtk_thread(problem_setters_do_not_call_back);
    }

    fn problem_setters_do_not_call_back() {
        let (to, heard) = recorder();
        let panel = ProblemsPanel::new(&to, |output| output);
        assert_eq!(panel.root.visible_child_name().as_deref(), Some("empty"));
        panel.show(&[report(3), report(9)], Some(1), true);
        settle();
        let list = list_of(&panel.root);
        assert_eq!(panel.root.visible_child_name().as_deref(), Some("list"));
        assert_eq!(list.selected_row().map(|row| row.index()), Some(1));
        panel.show_selected(Some(0));
        settle();
        assert_eq!(list.selected_row().map(|row| row.index()), Some(0));
        panel.show_selected(None);
        settle();
        assert!(list.selected_row().is_none());
        assert!(heard.borrow().is_empty(), "setters never call back");
    }

    #[test]
    fn a_choice_made_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(problem_choices_are_queued);
    }

    /// The old panel called its callback from inside GTK's
    /// `row-selected`, and the window answered by selecting again, which
    /// a hand-rolled flag kept from calling back. Now a choice only queues
    /// a message; the callback runs after the update has returned; a
    /// selection the window makes from inside it is queued and never calls
    /// back, and a place's link clicked from inside it is queued too.
    fn problem_choices_are_queued() {
        // The owner's handler: it selects again (by message, as the window
        // does) and a place's link is clicked from inside it.
        type Handles = (relm4::Sender<ProblemsInput>, gtk::Stack);
        let handles: Rc<RefCell<Option<Handles>>> = Rc::default();
        let heard: Rc<RefCell<Vec<String>>> = Rc::default();
        let (log, inner) = (Rc::clone(&heard), Rc::clone(&handles));
        let to = listener(move |output| {
            let index = match output {
                ProblemsOutput::Choose(index) => index,
                ProblemsOutput::Source(location) => {
                    log.borrow_mut().push(format!("source {}", location.line));
                    return;
                }
            };
            let Some((input, root)) = inner.borrow().clone() else {
                return;
            };
            let _ = input.send(ProblemsInput::ShowSelected(Some(0)));
            let link = list_in(root.upcast_ref())
                .and_then(|list| list.row_at_index(0))
                .and_then(|row| find_button(row.upcast_ref()));
            if let Some(link) = link {
                link.emit_clicked();
            }
            log.borrow_mut().push(format!("choose {index}"));
        });
        let panel = ProblemsPanel::new(&to, |output| output);
        handles.replace(Some((
            panel.controller.sender().clone(),
            panel.root.clone(),
        )));
        panel.show(&[report(3), report(9)], None, false);
        settle();
        let list = list_of(&panel.root);
        // People choose the second problem.
        list.select_row(list.row_at_index(1).as_ref());
        assert!(heard.borrow().is_empty(), "the choice is only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec!["choose 1".to_owned(), "source 3".to_owned()],
            "one choice (the window's selection never calls back), then the link"
        );
        assert_eq!(list.selected_row().map(|row| row.index()), Some(0));
    }

    #[test]
    fn a_cleared_selection_stays_cleared() {
        on_gtk_thread(cleared_selection_stays_cleared);
    }

    /// People clear the selection (Ctrl and a click): the panel tells
    /// nobody, and nothing it handles afterwards selects the row again.
    fn cleared_selection_stays_cleared() {
        let chosen: Rc<RefCell<Vec<usize>>> = Rc::default();
        let log = Rc::clone(&chosen);
        let to = listener(move |output| {
            if let ProblemsOutput::Choose(index) = output {
                log.borrow_mut().push(index);
            }
        });
        let panel = ProblemsPanel::new(&to, |output| output);
        panel.show(&[report(3), report(9)], None, false);
        settle();
        let list = list_of(&panel.root);
        list.select_row(list.row_at_index(1).as_ref());
        settle();
        assert_eq!(*chosen.borrow(), vec![1]);
        list.unselect_all();
        settle();
        assert!(list.selected_row().is_none(), "cleared by people");
        // Any later update (here a place's link clicked) leaves it so.
        let link = list
            .row_at_index(0)
            .and_then(|row| find_button(row.upcast_ref()));
        if let Some(link) = link {
            link.emit_clicked();
        }
        settle();
        assert!(list.selected_row().is_none(), "never selected again");
        assert_eq!(*chosen.borrow(), vec![1], "clearing tells nobody");
    }

    #[test]
    fn a_row_of_an_older_listing_chooses_nothing() {
        on_gtk_thread(older_rows_choose_nothing);
    }

    /// A row chosen while a new listing is queued belonged to the old
    /// problems: its number would mean a different problem in the new
    /// ones, so the choice is let go.
    fn older_rows_choose_nothing() {
        let chosen: Rc<RefCell<Vec<usize>>> = Rc::default();
        let log = Rc::clone(&chosen);
        let to = listener(move |output| {
            if let ProblemsOutput::Choose(index) = output {
                log.borrow_mut().push(index);
            }
        });
        let panel = ProblemsPanel::new(&to, |output| output);
        panel.show(&[report(3), report(9)], None, false);
        settle();
        let list = list_of(&panel.root);
        panel.show(&[report(4), report(5)], None, false);
        list.select_row(list.row_at_index(1).as_ref());
        settle();
        assert!(chosen.borrow().is_empty(), "the old row means nothing now");
        assert!(list.selected_row().is_none());
        list.select_row(list.row_at_index(1).as_ref());
        settle();
        assert_eq!(*chosen.borrow(), vec![1], "a row of the new listing does");
    }

    /// The first button inside a widget.
    fn find_button(widget: &gtk::Widget) -> Option<gtk::Button> {
        if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
            return Some(button);
        }
        std::iter::successors(widget.first_child(), gtk::Widget::next_sibling)
            .find_map(|child| find_button(&child))
    }

    #[test]
    fn a_dropped_files_list_is_freed() {
        on_gtk_thread(files_list_is_freed);
    }

    /// Dropping the list shuts its component down when the main loop next
    /// runs: the list, the model and the callback are all freed.
    fn files_list_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let to = listener(move |_: FilesListOutput| {
            let _ = &held;
        });
        let files = FilesList::new(&to, |output| output);
        drop(to);
        let callback = Rc::downgrade(&token);
        drop(token);
        files.set_folder(&cabinet());
        settle();
        let root = files.root.downgrade();
        let input = files.controller.sender().clone();
        drop(files);
        settle();
        assert!(root.upgrade().is_none(), "the list is freed");
        assert!(
            input
                .send(FilesListInput::Activated(cabinet().join("doors.stave")))
                .is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    /// People activate a row, as GTK tells the list.
    fn activate(list: &gtk::ListBox, row: &gtk::ListBoxRow) {
        list.emit_by_name::<()>("row-activated", &[row]);
    }

    #[test]
    fn a_file_opened_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(files_are_queued);
    }

    /// The old list read its paths from a `RefCell` inside GTK's
    /// `row-activated` and called its callback there; a callback that
    /// listed another folder replaced the paths under it. Now the folder
    /// is listed when the main loop runs, a row only queues a message
    /// naming its own file, the callback runs after the update, and a
    /// folder listed or a row activated from inside it is queued too. The
    /// row activated before the new folder's listing is handled still
    /// opens the cabinet file it showed, never one of the new folder's.
    fn files_are_queued() {
        let other = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/box");
        // The owner's handler, with the list's input and its root.
        type Handles = (relm4::Sender<FilesListInput>, gtk::Stack);
        let handles: Rc<RefCell<Option<Handles>>> = Rc::default();
        let heard: Rc<RefCell<Vec<PathBuf>>> = Rc::default();
        let (log, inner_handles) = (Rc::clone(&heard), Rc::clone(&handles));
        let inner = other.clone();
        let to = listener(move |FilesListOutput::Open(path)| {
            let Some((input, root)) = inner_handles.borrow().clone() else {
                return;
            };
            let first_call = log.borrow().is_empty();
            if first_call {
                // A folder with files of its own: queued, so the row below
                // is still the cabinet's when it is activated.
                let _ = input.send(FilesListInput::SetFolder(inner.clone()));
                if let Some(list) = list_in(root.upcast_ref())
                    && let Some(row) = list.row_at_index(0)
                {
                    activate(&list, &row);
                }
            }
            log.borrow_mut().push(path);
        });
        let files = FilesList::new(&to, |output| output);
        handles.replace(Some((
            files.controller.sender().clone(),
            files.root.clone(),
        )));
        files.set_folder(&cabinet());
        let list = list_of(&files.root);
        assert!(list.row_at_index(0).is_none(), "the listing is only queued");
        settle();
        let Some(first) = list.row_at_index(0) else {
            panic!("the cabinet has files");
        };
        activate(&list, &first);
        assert!(heard.borrow().is_empty(), "the choice is only queued");
        settle();
        let opened = heard.borrow().clone();
        assert_eq!(opened.len(), 2, "the outer row, then the inner one");
        assert!(
            opened.iter().all(|path| path.starts_with(cabinet())),
            "each row opened the cabinet file it showed: {opened:?}"
        );
        assert_eq!(opened.first(), opened.get(1), "both were the first row");
        // The new folder's rows open the new folder's files.
        let Some(row) = list.row_at_index(0) else {
            panic!("the box has files");
        };
        activate(&list, &row);
        settle();
        assert!(
            heard
                .borrow()
                .last()
                .is_some_and(|path| path.starts_with(&other)),
            "a row of the new listing opens its own file"
        );
    }
}
