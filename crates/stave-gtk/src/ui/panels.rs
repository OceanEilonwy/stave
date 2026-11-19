//! Two of the docked window's panels that are lists: Problems (every
//! problem the latest check found, in full, as `stave check` prints it:
//! the heading, each place with its line of source and the part of it
//! underlined, the notes and the help line; choosing one selects it, so
//! the 3D view poses it, and each place opens in the Code panel) and Files
//! (the project's `.stave` files; choosing one opens it in the Code
//! panel).

use stave_app::i18n::{fill_in, tr};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::inspect::Token;
use stave_app::problems::{Report, ReportPlace, Role};
use stave_app::session::SourceLocation;
use stave_lang::Severity;

use super::style::CODE;

type IndexCallback = Rc<dyn Fn(usize)>;
type PathCallback = Rc<dyn Fn(PathBuf)>;
type SourceCallback = Rc<dyn Fn(SourceLocation)>;

/// A list in a scrolled window, the way both panels show theirs, with a
/// page saying why it is empty in its place while it has no rows. `above`
/// goes over the list, scrolling with it.
fn scrolled_list(
    icon: &str,
    empty_title: &str,
    empty_text: &str,
    above: Option<&gtk::Widget>,
) -> (gtk::Stack, gtk::ListBox) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar", "stave-panel-list"])
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    if let Some(above) = above {
        content.append(above);
    }
    content.append(&list);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&content)
        .build();
    let empty = adw::StatusPage::builder()
        .icon_name(icon)
        .title(empty_title)
        .description(empty_text)
        .css_classes(["compact"])
        .build();
    let root = gtk::Stack::new();
    root.add_named(&scrolled, Some("list"));
    root.add_named(&empty, Some("empty"));
    root.set_visible_child_name("empty");
    (root, list)
}

/// Shows the list, or the page saying why it is empty.
fn show_rows(root: &gtk::Stack, any: bool) {
    root.set_visible_child_name(if any { "list" } else { "empty" });
}

/// The Problems panel.
pub struct ProblemsPanel {
    /// The panel's content.
    pub root: gtk::Stack,
    list: gtk::ListBox,
    /// Over the list while the 3D view shows an older design than the one
    /// these problems are about.
    stale: gtk::Label,
    callbacks: Rc<RefCell<ProblemCallbacks>>,
    /// Ignore selection changes made by the window itself.
    showing: std::cell::Cell<bool>,
    /// The problems the rows show, in order, so a new check replaces only
    /// the rows that changed.
    shown: RefCell<Vec<Report>>,
}

#[derive(Default)]
struct ProblemCallbacks {
    on_choose: Option<IndexCallback>,
    on_source: Option<SourceCallback>,
}

impl ProblemsPanel {
    /// An empty list.
    pub fn new() -> Rc<ProblemsPanel> {
        let stale = gtk::Label::builder()
            .label(tr(
                "The 3D view shows the last version without errors until these are fixed.",
            ))
            .xalign(0.0)
            .wrap(true)
            .visible(false)
            .css_classes(["dim", "caption", "stave-problems-stale"])
            .build();
        let (root, list) = scrolled_list(
            "object-select-symbolic",
            &tr("No problems"),
            &tr("Every part fits, every joint closes and everything that moves moves clear."),
            Some(stale.upcast_ref()),
        );
        let panel = Rc::new(ProblemsPanel {
            root,
            list,
            stale,
            callbacks: Rc::new(RefCell::new(ProblemCallbacks::default())),
            showing: std::cell::Cell::new(false),
            shown: RefCell::new(Vec::new()),
        });
        let weak = Rc::downgrade(&panel);
        panel.list.connect_row_selected(move |_, row| {
            let (Some(panel), Some(row)) = (weak.upgrade(), row) else {
                return;
            };
            if panel.showing.get() {
                return;
            }
            let callback = panel.callbacks.borrow().on_choose.clone();
            if let (Some(callback), Ok(index)) = (callback, usize::try_from(row.index())) {
                callback(index);
            }
        });
        panel
    }

    /// A problem was chosen (its index in the latest check's problems).
    pub fn connect_choose(&self, callback: impl Fn(usize) + 'static) {
        self.callbacks.borrow_mut().on_choose = Some(Rc::new(callback));
    }

    /// One of a problem's places was clicked, to open in the Code panel.
    pub fn connect_source(&self, callback: impl Fn(SourceLocation) + 'static) {
        self.callbacks.borrow_mut().on_source = Some(Rc::new(callback));
    }

    /// Lists these problems in full, the `selected` one marked. `stale`
    /// says the 3D view shows the last design that had no errors. Rows
    /// for problems that are the same at the start and the end of the
    /// list are kept; only the run between is replaced, so the scroll
    /// position and the keyboard's row survive a check that changed
    /// nothing, or little.
    pub fn show(&self, reports: &[Report], selected: Option<usize>, stale: bool) {
        self.showing.set(true);
        {
            let mut shown = self.shown.borrow_mut();
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
                if let Some(row) = self.list.row_at_index(at) {
                    self.list.remove(&row);
                }
            }
            let added = reports.len().saturating_sub(prefix).saturating_sub(suffix);
            for (offset, report) in reports.iter().skip(prefix).take(added).enumerate() {
                let position = prefix
                    .checked_add(offset)
                    .and_then(|position| i32::try_from(position).ok())
                    .unwrap_or(-1);
                self.list.insert(&self.report_row(report), position);
            }
            *shown = reports.to_vec();
        }
        self.stale.set_visible(stale && !reports.is_empty());
        show_rows(&self.root, !reports.is_empty());
        self.show_selected(selected);
        self.showing.set(false);
    }

    /// Marks the selected problem, or none.
    pub fn show_selected(&self, selected: Option<usize>) {
        self.showing.set(true);
        let row = selected
            .and_then(|index| i32::try_from(index).ok())
            .and_then(|index| self.list.row_at_index(index));
        self.list.select_row(row.as_ref());
        self.showing.set(false);
    }

    /// One problem as `stave check` prints it: `error[require]: message`,
    /// each place with its line of source, then the notes and the help.
    fn report_row(&self, report: &Report) -> gtk::Box {
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
            text.append(&self.place_block(place, report.severity));
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
    fn place_block(&self, place: &ReportPlace, severity: Severity) -> gtk::Box {
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
        let callbacks = self.callbacks.clone();
        let location = place.location.clone();
        link.connect_clicked(move |_| {
            let callback = callbacks.borrow().on_source.clone();
            if let Some(callback) = callback {
                callback(location.clone());
            }
        });
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

/// The Files panel.
pub struct FilesList {
    /// The panel's content.
    pub root: gtk::Stack,
    list: gtk::ListBox,
    paths: RefCell<Vec<PathBuf>>,
    on_open: RefCell<Option<PathCallback>>,
}

impl FilesList {
    /// An empty list.
    pub fn new() -> Rc<FilesList> {
        let (root, list) = scrolled_list(
            "folder-symbolic",
            &tr("No files"),
            &tr("This folder has no .stave files."),
            None,
        );
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_activate_on_single_click(true);
        // Compact rows, like the Parts panel's (style.rs, SHELL).
        list.add_css_class("stave-files-list");
        let files = Rc::new(FilesList {
            root,
            list,
            paths: RefCell::new(Vec::new()),
            on_open: RefCell::new(None),
        });
        let weak = Rc::downgrade(&files);
        files.list.connect_row_activated(move |_, row| {
            let Some(files) = weak.upgrade() else {
                return;
            };
            let path = usize::try_from(row.index())
                .ok()
                .and_then(|index| files.paths.borrow().get(index).cloned());
            let callback = files.on_open.borrow().clone();
            if let (Some(callback), Some(path)) = (callback, path) {
                callback(path);
            }
        });
        files
    }

    /// A file was chosen.
    pub fn connect_open(&self, callback: impl Fn(PathBuf) + 'static) {
        self.on_open.replace(Some(Rc::new(callback)));
    }

    /// Lists the `.stave` files in a project folder and the folders in it
    /// (not hidden ones, nor `target`), sorted by path.
    pub fn set_folder(&self, folder: &Path) {
        let mut paths = Vec::new();
        collect_stave_files(folder, &mut paths);
        paths.sort();
        self.list.remove_all();
        for path in &paths {
            let shown = path.strip_prefix(folder).unwrap_or(path);
            let label = gtk::Label::builder()
                .label(shown.display().to_string())
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::Middle)
                .build();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&gtk::Image::from_icon_name("text-x-generic-symbolic"));
            row.append(&label);
            self.list.append(&row);
        }
        show_rows(&self.root, !paths.is_empty());
        self.paths.replace(paths);
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
    use super::*;

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
    fn rows(panel: &ProblemsPanel) -> Vec<gtk::ListBoxRow> {
        (0..)
            .map_while(|index| panel.list.row_at_index(index))
            .collect()
    }

    #[test]
    fn a_new_check_replaces_only_the_problems_that_changed() {
        crate::ui::gtk_test::on_gtk_thread(problems_update_in_place);
    }

    fn problems_update_in_place() {
        let panel = ProblemsPanel::new();
        let first = vec![problem("a"), problem("b"), problem("c")];
        panel.show(&first, None, false);
        let before = rows(&panel);
        assert_eq!(before.len(), 3);
        panel.show(&first, None, false);
        assert_eq!(rows(&panel), before, "the same problems keep their rows");
        let second = vec![problem("a"), problem("x"), problem("y"), problem("c")];
        panel.show(&second, Some(2), false);
        let after = rows(&panel);
        assert_eq!(after.len(), 4);
        assert_eq!(after.first(), before.first(), "a is kept");
        assert_eq!(after.last(), before.last(), "c is kept");
        assert!(!after.contains(&before[1]), "b is replaced");
        assert_eq!(
            panel.list.selected_row().map(|row| row.index()),
            Some(2),
            "the selected problem is marked"
        );
        panel.show(&[], None, false);
        assert!(rows(&panel).is_empty());
    }

    #[test]
    fn the_cabinet_lists_its_own_files() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
        let mut paths = Vec::new();
        collect_stave_files(&folder, &mut paths);
        assert!(paths.iter().any(|p| p.ends_with("doors.stave")));
        assert!(
            paths
                .iter()
                .all(|p| p.extension().is_some_and(|e| e == "stave"))
        );
    }
}
