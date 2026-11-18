//! Two of the docked window's panels that are plain lists: Problems (every
//! problem the latest check found; choosing one selects it, so the 3D view
//! poses it and Details explains it) and Files (the project's `.stave`
//! files; choosing one opens it in the Code panel).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::view::Problem;
use stave_lang::Severity;

type IndexCallback = Rc<dyn Fn(usize)>;
type PathCallback = Rc<dyn Fn(PathBuf)>;

/// A list in a scrolled window, the way both panels show theirs, with a
/// page saying why it is empty in its place while it has no rows.
fn scrolled_list(icon: &str, empty_title: &str, empty_text: &str) -> (gtk::Stack, gtk::ListBox) {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar", "stave-panel-list"])
        .build();
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
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
    on_choose: RefCell<Option<IndexCallback>>,
    /// Ignore selection changes made by the window itself.
    showing: std::cell::Cell<bool>,
}

impl ProblemsPanel {
    /// An empty list.
    pub fn new() -> Rc<ProblemsPanel> {
        let (root, list) = scrolled_list(
            "object-select-symbolic",
            "No problems",
            "Every part fits, every joint closes and everything that moves moves clear.",
        );
        let panel = Rc::new(ProblemsPanel {
            root,
            list,
            on_choose: RefCell::new(None),
            showing: std::cell::Cell::new(false),
        });
        let weak = Rc::downgrade(&panel);
        panel.list.connect_row_selected(move |_, row| {
            let (Some(panel), Some(row)) = (weak.upgrade(), row) else {
                return;
            };
            if panel.showing.get() {
                return;
            }
            let callback = panel.on_choose.borrow().clone();
            if let (Some(callback), Ok(index)) = (callback, usize::try_from(row.index())) {
                callback(index);
            }
        });
        panel
    }

    /// A problem was chosen (its index in the latest check's problems).
    pub fn connect_choose(&self, callback: impl Fn(usize) + 'static) {
        self.on_choose.replace(Some(Rc::new(callback)));
    }

    /// Lists these problems, the `selected` one marked.
    pub fn show(&self, problems: &[Problem], selected: Option<usize>) {
        self.showing.set(true);
        self.list.remove_all();
        for problem in problems {
            self.list.append(&problem_row(problem));
        }
        show_rows(&self.root, !problems.is_empty());
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
}

fn problem_row(problem: &Problem) -> gtk::Box {
    let (icon, class) = match problem.severity {
        Severity::Error => ("dialog-error-symbolic", "error"),
        Severity::Warning => ("dialog-warning-symbolic", "warning"),
        Severity::Note => ("dialog-information-symbolic", "dim"),
    };
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .css_classes(["stave-problem-row"])
        .build();
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class(class);
    image.set_valign(gtk::Align::Start);
    row.append(&image);
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .hexpand(true)
        .build();
    text.append(
        &gtk::Label::builder()
            .label(&problem.message)
            .xalign(0.0)
            .wrap(true)
            .build(),
    );
    if let Some(location) = &problem.location {
        text.append(
            &gtk::Label::builder()
                .label(location)
                .xalign(0.0)
                .css_classes(["dim", "caption"])
                .build(),
        );
    }
    row.append(&text);
    row
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
            "No files",
            "This folder has no .stave files.",
        );
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_activate_on_single_click(true);
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
