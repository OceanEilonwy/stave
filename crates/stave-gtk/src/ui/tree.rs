//! The model tree, floating over the 3D view (V2C_Home.dc.html): the
//! carcass's parts, then each assembly with its motion, its parts and
//! hardware, each row with a dot for its worst problem
//! (`stave_app::problems::marks`), and under the selected part the joints
//! it is in (`stave_app::focus::targets_of`), which open focus mode. Rows
//! come from `stave_app::inspect::parts_tree`; activating one asks the
//! window to select it. With no parts it says why (`stave_app::empty`),
//! and for a library folder lists what the library defines; activating
//! one of those opens its text.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::empty::{EmptyState, LibraryItem};
use stave_app::focus::Target;
use stave_app::inspect::{AssemblyNode, PartsTree, Swatch, SwatchShape, TreeEntry};
use stave_app::problems::Marks;
use stave_app::session::{Selection, SourceLocation};
use stave_model::solved::AssemblyId;

use super::icons::{self, PathIcon};

/// Called with what a row stands for when it is activated.
type SelectCallback = Rc<dyn Fn(Selection)>;
/// Called with where a library's item is written when it is activated.
type SourceCallback = Rc<dyn Fn(SourceLocation)>;
/// Called with a joint to focus on when its row is activated.
type FocusCallback = Rc<dyn Fn(Target)>;

/// What a row of the list is.
#[derive(Debug, Clone)]
enum RowKind {
    Heading,
    Entry {
        /// What activating it selects: its first item.
        selection: Selection,
        /// Everything it stands for, so selecting any of them in the 3D
        /// view highlights it.
        items: Vec<stave_scene::Item>,
    },
    Assembly(AssemblyId),
    /// A joint the selected part is in: opens focus mode.
    Joint(Target),
    /// Something a library defines: where it is written.
    Source(SourceLocation),
}

/// The model tree's floating panel.
pub struct TreeSidebar {
    /// The panel, to float over the 3D view.
    pub root: gtk::Box,
    /// Folds the panel down to its heading, or opens it again.
    pub collapse: gtk::Button,
    list: gtk::ListBox,
    body: gtk::Box,
    state: Rc<RefCell<TreeState>>,
}

#[derive(Default)]
struct TreeState {
    tree: PartsTree,
    rows: Vec<(gtk::ListBoxRow, RowKind)>,
    /// Assemblies the user opened (`true`) or shut (`false`); the others
    /// are open when they hold the selection.
    toggled: BTreeMap<AssemblyId, bool>,
    selection: Option<Selection>,
    /// The latest check's marks, for the dots.
    marks: Marks,
    /// The joints the selected part is in, shown under its row.
    joints: Vec<(Target, String)>,
    /// Why there are no parts, when there are none.
    empty: Option<EmptyState>,
    on_select: Option<SelectCallback>,
    on_source: Option<SourceCallback>,
    on_focus: Option<FocusCallback>,
    /// Ignore row selection changes made while filling or syncing.
    syncing: bool,
}

fn rgba(swatch: Swatch) -> gtk::gdk::RGBA {
    gtk::gdk::RGBA::new(
        f32::from(swatch.colour.red) / 255.0,
        f32::from(swatch.colour.green) / 255.0,
        f32::from(swatch.colour.blue) / 255.0,
        1.0,
    )
}

impl TreeSidebar {
    /// An empty sidebar.
    pub fn new() -> TreeSidebar {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["stave-tree"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Model")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(520)
            .child(&list)
            .build();
        let heading = gtk::Label::builder()
            .label("MODEL")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["dim-label"])
            .build();
        let collapse = gtk::Button::builder()
            .icon_name("go-up-symbolic")
            .tooltip_text("Fold the model tree (F9)")
            .css_classes(["flat", "circular", "stave-close"])
            .build();
        let head = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-model-head"])
            .build();
        head.append(&heading);
        head.append(&collapse);
        let note = gtk::Label::builder()
            .label("Click a row and the camera frames it. Find files with Ctrl+K.")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "stave-model-note"])
            .build();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        body.append(&scrolled);
        body.append(&note);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-panel", "stave-model"])
            .width_request(240)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .build();
        root.append(&head);
        root.append(&body);
        let sidebar = TreeSidebar {
            root,
            collapse,
            list,
            body,
            state: Rc::new(RefCell::new(TreeState::default())),
        };
        sidebar.connect();
        sidebar
    }

    fn connect(&self) {
        let body = self.body.clone();
        self.collapse.connect_clicked(move |button| {
            let open = !body.is_visible();
            body.set_visible(open);
            button.set_icon_name(if open {
                "go-up-symbolic"
            } else {
                "go-down-symbolic"
            });
            button.set_tooltip_text(Some(if open {
                "Fold the model tree (F9)"
            } else {
                "Open the model tree (F9)"
            }));
        });
        let state = self.state.clone();
        let list = self.list.clone();
        self.list.connect_row_activated(move |_, row| {
            let kind = {
                let state = state.borrow();
                state
                    .rows
                    .iter()
                    .find(|(r, _)| r == row)
                    .map(|(_, kind)| kind.clone())
            };
            match kind {
                Some(RowKind::Entry { selection, .. }) => {
                    let callback = state.borrow().on_select.clone();
                    if let Some(callback) = callback {
                        callback(selection);
                    }
                }
                Some(RowKind::Assembly(id)) => {
                    {
                        let mut state = state.borrow_mut();
                        let open = state
                            .tree
                            .assemblies
                            .iter()
                            .find_map(|node| find(node, id))
                            .is_some_and(|node| {
                                is_open(node, &state.toggled, state.selection.as_ref())
                            });
                        state.toggled.insert(id, !open);
                    }
                    fill(&list, &state);
                    let callback = state.borrow().on_select.clone();
                    if let Some(callback) = callback {
                        callback(Selection::Assembly(id));
                    }
                }
                Some(RowKind::Joint(target)) => {
                    let callback = state.borrow().on_focus.clone();
                    if let Some(callback) = callback {
                        callback(target);
                    }
                }
                Some(RowKind::Source(location)) => {
                    let callback = state.borrow().on_source.clone();
                    if let Some(callback) = callback {
                        callback(location);
                    }
                }
                Some(RowKind::Heading) | None => {}
            }
        });
        // Arrow keys move the row selection; follow it like a click.
        let state = self.state.clone();
        self.list.connect_row_selected(move |_, row| {
            let Some(row) = row else { return };
            let (syncing, kind) = {
                // Busy: the tree itself is changing the rows.
                let Ok(state) = state.try_borrow() else {
                    return;
                };
                let kind = state
                    .rows
                    .iter()
                    .find(|(r, _)| r == row)
                    .map(|(_, kind)| kind.clone());
                (state.syncing, kind)
            };
            if syncing {
                return;
            }
            if let Some(RowKind::Entry { selection, .. }) = kind {
                let callback = state.borrow().on_select.clone();
                if let Some(callback) = callback {
                    callback(selection);
                }
            }
        });
    }

    /// Calls `callback` with what a row stands for when it is chosen.
    pub fn connect_select(&self, callback: impl Fn(Selection) + 'static) {
        self.state.borrow_mut().on_select = Some(Rc::new(callback));
    }

    /// Calls `callback` with where a library's item is written when its
    /// row is chosen.
    pub fn connect_source(&self, callback: impl Fn(SourceLocation) + 'static) {
        self.state.borrow_mut().on_source = Some(Rc::new(callback));
    }

    /// Calls `callback` with a joint to focus on when its row is chosen.
    pub fn connect_focus(&self, callback: impl Fn(Target) + 'static) {
        self.state.borrow_mut().on_focus = Some(Rc::new(callback));
    }

    /// Shows a new tree, keeping what is shut.
    pub fn show_tree(&self, tree: PartsTree) {
        {
            let mut state = self.state.borrow_mut();
            state.tree = tree;
            state.empty = None;
        }
        fill(&self.list, &self.state);
    }

    /// Shows why there are no parts; for a library, what it defines.
    pub fn show_empty(&self, empty: EmptyState) {
        {
            let mut state = self.state.borrow_mut();
            state.tree = PartsTree::default();
            state.empty = Some(empty);
        }
        fill(&self.list, &self.state);
    }

    /// Takes the latest check's marks for the dots.
    pub fn show_marks(&self, marks: Marks) {
        self.state.borrow_mut().marks = marks;
        fill(&self.list, &self.state);
    }

    /// Highlights the selection's row and lists the joints the selected
    /// part is in under it.
    pub fn show_selection(&self, selection: Option<&Selection>, joints: Vec<(Target, String)>) {
        {
            let mut state = self.state.borrow_mut();
            state.selection = selection.cloned();
            state.joints = joints;
        }
        // The assembly holding it opens.
        fill(&self.list, &self.state);
    }

    /// Folds the panel down to its heading, or opens it again (F9).
    pub fn toggle(&self) {
        self.collapse.emit_clicked();
    }
}

/// Rebuilds the rows.
fn fill(list: &gtk::ListBox, state: &Rc<RefCell<TreeState>>) {
    let mut state_ref = state.borrow_mut();
    state_ref.syncing = true;
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
    let tree = &state_ref.tree;
    let shown = Shown {
        selection: state_ref.selection.as_ref(),
        marks: &state_ref.marks,
        joints: &state_ref.joints,
    };
    let mut rows = Vec::new();
    if !tree.carcass.is_empty() {
        rows.push(heading("CARCASS", true));
        for entry in &tree.carcass {
            entry_rows(entry, 0, &shown, &mut rows);
        }
    }
    if !tree.assemblies.is_empty() {
        rows.push(heading("ASSEMBLIES", tree.carcass.is_empty()));
        for node in &tree.assemblies {
            assembly_rows(node, 0, &state_ref.toggled, &shown, &mut rows);
        }
    }
    if rows.is_empty()
        && let Some(empty) = &state_ref.empty
    {
        empty_rows(empty, &mut rows);
    }
    if rows.is_empty() {
        let label = gtk::Label::builder()
            .label("Nothing to show yet")
            .css_classes(["dim-label"])
            .margin_top(12)
            .build();
        let row = gtk::ListBoxRow::builder()
            .child(&label)
            .activatable(false)
            .selectable(false)
            .build();
        rows.push((row, RowKind::Heading));
    }
    for (row, _) in &rows {
        list.append(row);
    }
    state_ref.rows = rows;
    drop(state_ref);
    sync_selection(list, state);
    state.borrow_mut().syncing = false;
}

fn sync_selection(list: &gtk::ListBox, state: &Rc<RefCell<TreeState>>) {
    let mut state = state.borrow_mut();
    let was = state.syncing;
    state.syncing = true;
    let selected = state.selection.as_ref().and_then(|selection| {
        state.rows.iter().find_map(|(row, kind)| match kind {
            RowKind::Entry { items, .. }
                if selection.item().is_some_and(|item| items.contains(&item)) =>
            {
                Some(row.clone())
            }
            RowKind::Assembly(id) if *selection == Selection::Assembly(*id) => Some(row.clone()),
            RowKind::Source(location) if *selection == Selection::Location(location.clone()) => {
                Some(row.clone())
            }
            RowKind::Joint(target) if *selection == Selection::of_target(*target) => {
                Some(row.clone())
            }
            RowKind::Entry { .. }
            | RowKind::Assembly(_)
            | RowKind::Joint(_)
            | RowKind::Source(_)
            | RowKind::Heading => None,
        })
    });
    match selected {
        Some(row) => list.select_row(Some(&row)),
        None => list.unselect_all(),
    }
    state.syncing = was;
}

/// What the rows show besides the tree itself.
struct Shown<'a> {
    selection: Option<&'a Selection>,
    marks: &'a Marks,
    joints: &'a [(Target, String)],
}

/// The rows that say why there are no parts: a line, then for a library
/// what it defines, by group.
fn empty_rows(empty: &EmptyState, rows: &mut Vec<(gtk::ListBoxRow, RowKind)>) {
    {
        let label = gtk::Label::builder()
            .label(empty.sidebar())
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label", "stave-sidebar-note"])
            .build();
        rows.push((
            gtk::ListBoxRow::builder()
                .child(&label)
                .activatable(false)
                .selectable(false)
                .build(),
            RowKind::Heading,
        ));
    }
    let Some(library) = &empty.library else {
        return;
    };
    for (group, items) in &library.groups {
        let shown: Vec<&LibraryItem> = items.iter().collect();
        if shown.is_empty() {
            continue;
        }
        rows.push(heading(group.heading(), false));
        for item in shown {
            rows.push(library_row(item));
        }
    }
}

/// A row for something a library defines: its name and the first line of
/// its doc comment.
fn library_row(item: &LibraryItem) -> (gtk::ListBoxRow, RowKind) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(1)
        .css_classes(["stave-entry", "stave-library-entry"])
        .build();
    content.append(
        &gtk::Label::builder()
            .label(&item.name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    if let Some(summary) = &item.summary {
        content.append(
            &gtk::Label::builder()
                .label(summary)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["stave-detail"])
                .build(),
        );
    }
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(&item.name)]);
    let kind = match &item.location {
        Some(location) => {
            row.set_tooltip_text(Some(item.summary.as_deref().unwrap_or("Open its text")));
            RowKind::Source(location.clone())
        }
        None => {
            row.set_activatable(false);
            row.set_selectable(false);
            RowKind::Heading
        }
    };
    (row, kind)
}

fn heading(text: &str, first: bool) -> (gtk::ListBoxRow, RowKind) {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["stave-heading"])
        .build();
    if first {
        label.add_css_class("first");
    }
    let row = gtk::ListBoxRow::builder()
        .child(&label)
        .activatable(false)
        .selectable(false)
        .build();
    (row, RowKind::Heading)
}

/// The rows of an entry: one, or one per member when the selection is
/// one of its members (so each can be chosen).
fn entry_rows(
    entry: &TreeEntry,
    depth: usize,
    shown: &Shown<'_>,
    rows: &mut Vec<(gtk::ListBoxRow, RowKind)>,
) {
    let selected = shown.selection.and_then(Selection::item);
    let split = entry.items.len() > 1
        && selected.is_some_and(|item| entry.items.iter().any(|i| *i == item));
    if split {
        for ((item, label), detail) in entry
            .items
            .iter()
            .zip(&entry.member_labels)
            .zip(&entry.member_details)
        {
            let mark = shown.marks.item(*item);
            rows.push(entry_row(
                entry,
                label,
                detail.as_deref(),
                vec![*item],
                depth,
                mark,
            ));
            if selected == Some(*item) {
                joint_rows(shown.joints, depth, rows);
            }
        }
    } else {
        rows.push(entry_row(
            entry,
            &entry.label,
            entry.detail.as_deref(),
            entry.items.iter().copied().collect(),
            depth,
            shown.marks.entry(entry),
        ));
        if selected.is_some_and(|item| entry.items.iter().any(|i| *i == item)) {
            joint_rows(shown.joints, depth, rows);
        }
    }
}

/// The joints the selected part is in, under its row; each opens focus
/// mode.
fn joint_rows(
    joints: &[(Target, String)],
    depth: usize,
    rows: &mut Vec<(gtk::ListBoxRow, RowKind)>,
) {
    for (target, title) in joints {
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-joint"])
            .margin_start(i32::try_from(depth.saturating_mul(14)).unwrap_or(0))
            .build();
        let word = match target {
            Target::Joint { .. } => "JOINT",
            Target::Hardware(_) => "WORKS",
        };
        content.append(
            &gtk::Label::builder()
                .label(word)
                .css_classes(["stave-joint-word"])
                .build(),
        );
        content.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        let row = gtk::ListBoxRow::builder()
            .child(&content)
            .tooltip_text(format!("Focus on {title}"))
            .build();
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "Focus on {title}"
        ))]);
        rows.push((row, RowKind::Joint(*target)));
    }
}

fn entry_row(
    entry: &TreeEntry,
    label: &str,
    detail: Option<&str>,
    items: Vec<stave_scene::Item>,
    depth: usize,
    mark: Option<stave_app::problems::Mark>,
) -> (gtk::ListBoxRow, RowKind) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["stave-entry"])
        .build();
    match depth {
        0 => {}
        1 => content.add_css_class("nested"),
        _ => content.add_css_class("nested2"),
    }
    content.append(&icons::swatch(
        rgba(entry.swatch),
        entry.swatch.shape == SwatchShape::Round,
        10,
    ));
    content.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    if let Some(detail) = detail {
        content.append(
            &gtk::Label::builder()
                .label(detail)
                .css_classes(["stave-detail"])
                .build(),
        );
    }
    content.append(&super::style::mark_dot(mark));
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(label)]);
    let first = items.first().copied().unwrap_or(*entry.items.first());
    (
        row,
        RowKind::Entry {
            selection: Selection::of_item(first),
            items,
        },
    )
}

fn assembly_rows(
    node: &AssemblyNode,
    depth: usize,
    toggled: &BTreeMap<AssemblyId, bool>,
    shown: &Shown<'_>,
    rows: &mut Vec<(gtk::ListBoxRow, RowKind)>,
) {
    let open = is_open(node, toggled, shown.selection);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["stave-assembly"])
        .build();
    if depth > 0 {
        content.add_css_class("nested");
    }
    content.append(&PathIcon::new(
        if open {
            icons::EXPANDED
        } else {
            icons::COLLAPSED
        },
        12,
    ));
    content.append(
        &gtk::Label::builder()
            .label(&node.label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    if let Some(detail) = &node.detail {
        content.append(
            &gtk::Label::builder()
                .label(detail)
                .css_classes(["stave-detail"])
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
    }
    content.append(&super::style::mark_dot(shown.marks.assembly(node.id)));
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(&node.label)]);
    row.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
    rows.push((row, RowKind::Assembly(node.id)));
    if !open {
        return;
    }
    for entry in &node.entries {
        entry_rows(entry, depth.saturating_add(1), shown, rows);
    }
    for child in &node.children {
        assembly_rows(child, depth.saturating_add(1), toggled, shown, rows);
    }
}

/// The node for an assembly, at or below `node`.
fn find(node: &AssemblyNode, id: AssemblyId) -> Option<&AssemblyNode> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, id))
}

/// Whether an assembly's rows show: as the user left it, or open when it
/// holds the selection.
fn is_open(
    node: &AssemblyNode,
    toggled: &BTreeMap<AssemblyId, bool>,
    selection: Option<&Selection>,
) -> bool {
    toggled
        .get(&node.id)
        .copied()
        .unwrap_or_else(|| selection.is_some_and(|s| holds(node, s)))
}

/// Whether the selection is inside an assembly.
fn holds(node: &AssemblyNode, selection: &Selection) -> bool {
    *selection == Selection::Assembly(node.id) && !node.entries.is_empty()
        || node.entries.iter().any(|entry| {
            selection
                .item()
                .is_some_and(|item| entry.items.iter().any(|i| *i == item))
        })
        || node.children.iter().any(|child| holds(child, selection))
}
