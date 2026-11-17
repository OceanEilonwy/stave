//! The Parts sidebar: a search field and the parts tree (the carcass's
//! parts, then each assembly with its motion, its parts and hardware).
//! Rows come from `stave_app::inspect::parts_tree`; activating one asks
//! the window to select it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::inspect::{AssemblyNode, PartsTree, Swatch, SwatchShape, TreeEntry};
use stave_app::session::Selection;
use stave_model::solved::AssemblyId;

use super::icons::{self, PathIcon};

/// Called with what a row stands for when it is activated.
type SelectCallback = Rc<dyn Fn(Selection)>;

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
}

/// The Parts sidebar.
pub struct TreeSidebar {
    /// The sidebar, ready to put in the split view.
    pub root: gtk::Box,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
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
    on_select: Option<SelectCallback>,
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
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Find a part")
            .css_classes(["stave-search"])
            .build();
        search.update_property(&[gtk::accessible::Property::Label("Find a part")]);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["stave-tree"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Parts")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["stave-sidebar"])
            .width_request(240)
            .build();
        root.append(&search);
        root.append(&scrolled);
        let sidebar = TreeSidebar {
            root,
            search,
            list,
            state: Rc::new(RefCell::new(TreeState::default())),
        };
        sidebar.connect();
        sidebar
    }

    fn connect(&self) {
        let state = self.state.clone();
        let list = self.list.clone();
        let search = self.search.clone();
        self.search.connect_search_changed(move |_| {
            fill(&list, &state, &search.text());
        });
        let state = self.state.clone();
        let list = self.list.clone();
        let search = self.search.clone();
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
                    fill(&list, &state, &search.text());
                    let callback = state.borrow().on_select.clone();
                    if let Some(callback) = callback {
                        callback(Selection::Assembly(id));
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

    /// Shows a new tree, keeping the search and what is shut.
    pub fn show_tree(&self, tree: PartsTree) {
        self.state.borrow_mut().tree = tree;
        fill(&self.list, &self.state, &self.search.text());
    }

    /// Highlights the selection's row.
    pub fn show_selection(&self, selection: Option<&Selection>) {
        self.state.borrow_mut().selection = selection.cloned();
        // The assembly holding it opens.
        fill(&self.list, &self.state, &self.search.text());
    }

    /// Focuses the search field.
    pub fn focus_search(&self) {
        self.search.grab_focus();
    }
}

/// Rebuilds the rows for a search.
fn fill(list: &gtk::ListBox, state: &Rc<RefCell<TreeState>>, query: &str) {
    let mut state_ref = state.borrow_mut();
    state_ref.syncing = true;
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
    let tree = state_ref.tree.filtered(query);
    let searching = !query.trim().is_empty();
    let mut rows = Vec::new();
    if !tree.carcass.is_empty() {
        rows.push(heading("CARCASS", true));
        for entry in &tree.carcass {
            entry_rows(entry, 0, state_ref.selection.as_ref(), &mut rows);
        }
    }
    if !tree.assemblies.is_empty() {
        rows.push(heading("ASSEMBLIES", tree.carcass.is_empty()));
        for node in &tree.assemblies {
            assembly_rows(
                node,
                0,
                &state_ref.toggled,
                state_ref.selection.as_ref(),
                searching,
                &mut rows,
            );
        }
    }
    if rows.is_empty() {
        let label = gtk::Label::builder()
            .label(if searching {
                "No part has that name"
            } else {
                "Nothing to show yet"
            })
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
            RowKind::Entry { .. } | RowKind::Assembly(_) | RowKind::Heading => None,
        })
    });
    match selected {
        Some(row) => list.select_row(Some(&row)),
        None => list.unselect_all(),
    }
    state.syncing = was;
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
    selection: Option<&Selection>,
    rows: &mut Vec<(gtk::ListBoxRow, RowKind)>,
) {
    let selected = selection.and_then(Selection::item);
    let split = entry.items.len() > 1
        && selected.is_some_and(|item| entry.items.iter().any(|i| *i == item));
    if split {
        for (item, label) in entry.items.iter().zip(&entry.member_labels) {
            rows.push(entry_row(entry, label, vec![*item], depth));
        }
    } else {
        rows.push(entry_row(
            entry,
            &entry.label,
            entry.items.iter().copied().collect(),
            depth,
        ));
    }
}

fn entry_row(
    entry: &TreeEntry,
    label: &str,
    items: Vec<stave_scene::Item>,
    depth: usize,
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
    if let Some(detail) = &entry.detail {
        content.append(
            &gtk::Label::builder()
                .label(detail)
                .css_classes(["stave-detail"])
                .build(),
        );
    }
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
    selection: Option<&Selection>,
    searching: bool,
    rows: &mut Vec<(gtk::ListBoxRow, RowKind)>,
) {
    let open = searching || is_open(node, toggled, selection);
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
    let row = gtk::ListBoxRow::builder().child(&content).build();
    row.update_property(&[gtk::accessible::Property::Label(&node.label)]);
    row.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
    rows.push((row, RowKind::Assembly(node.id)));
    if !open {
        return;
    }
    for entry in &node.entries {
        entry_rows(entry, depth.saturating_add(1), selection, rows);
    }
    for child in &node.children {
        assembly_rows(
            child,
            depth.saturating_add(1),
            toggled,
            selection,
            searching,
            rows,
        );
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
