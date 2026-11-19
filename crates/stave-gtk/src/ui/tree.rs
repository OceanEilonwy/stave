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

/// The Parts panel's tree.
pub struct TreeSidebar {
    /// The panel's content.
    pub root: gtk::Box,
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
    /// The latest check's marks, for the dots.
    marks: Rc<Marks>,
    /// The part whose joints are listed under its row, and those joints.
    listing: Listing,
    /// The joint row being activated, while its callback runs: the
    /// selection it makes keeps the joints listed where it was clicked.
    clicked: Option<Target>,
    /// Why there are no parts, when there are none.
    empty: Option<EmptyState>,
    /// What an empty tree says instead of "Nothing to show yet" (a new
    /// project's envelope: where to click).
    placeholder: Option<String>,
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
            .vexpand(true)
            .child(&list)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-model"])
            .build();
        root.append(&scrolled);
        let sidebar = TreeSidebar {
            root,
            list,
            state: Rc::new(RefCell::new(TreeState::default())),
        };
        sidebar.connect();
        sidebar
    }

    fn connect(&self) {
        let state = self.state.clone();
        // The list the signal passes, not a captured one: the list keeps
        // its handlers, so a capture would keep it forever.
        self.list.connect_row_activated(move |list, row| {
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
                                is_open(
                                    node,
                                    &state.toggled,
                                    state.selection.as_ref(),
                                    state.listing.under,
                                )
                            });
                        state.toggled.insert(id, !open);
                    }
                    fill(list, &state);
                    let callback = state.borrow().on_select.clone();
                    if let Some(callback) = callback {
                        callback(Selection::Assembly(id));
                    }
                }
                Some(RowKind::Joint(target)) => {
                    let callback = state.borrow().on_focus.clone();
                    if let Some(callback) = callback {
                        state.borrow_mut().clicked = Some(target);
                        callback(target);
                        state.borrow_mut().clicked = None;
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
                // The tree changes the rows with nothing borrowed and with
                // `syncing` up.
                let state = state.borrow();
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

    /// What the tree says while it has no rows: `None` for "Nothing to
    /// show yet".
    pub fn set_placeholder(&self, placeholder: Option<&str>) {
        let changed = {
            let mut state = self.state.borrow_mut();
            let changed = state.placeholder.as_deref() != placeholder;
            state.placeholder = placeholder.map(str::to_owned);
            changed
        };
        if changed {
            fill(&self.list, &self.state);
        }
    }

    /// Takes the latest check's marks for the dots.
    pub fn show_marks(&self, marks: Rc<Marks>) {
        self.state.borrow_mut().marks = marks;
        fill(&self.list, &self.state);
    }

    /// Highlights the selection's row and lists the joints the selected
    /// part is in under it. Choosing one of those joints keeps them listed
    /// under the same part, and then the rows are left as they are, so
    /// nothing shuts and the list does not move.
    pub fn show_selection(&self, selection: Option<&Selection>, joints: Vec<(Target, String)>) {
        let same_rows = {
            let mut state = self.state.borrow_mut();
            let listing = state.listing.next(selection, joints, state.clicked);
            let was_open = open_assemblies(
                &state.tree.assemblies,
                &state.toggled,
                state.selection.as_ref(),
                state.listing.under,
            );
            let now_open = open_assemblies(
                &state.tree.assemblies,
                &state.toggled,
                selection,
                listing.under,
            );
            let same_rows = listing == state.listing && was_open == now_open;
            state.selection = selection.cloned();
            state.listing = listing;
            same_rows
        };
        if same_rows {
            sync_selection(&self.list, &self.state);
        } else {
            // The assembly holding it opens.
            fill(&self.list, &self.state);
        }
    }
}

/// The part whose joints the tree lists under its row, and those joints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Listing {
    /// The part (or piece of hardware, or member) they are under.
    under: Option<stave_scene::Item>,
    /// The joints, with their titles.
    joints: Vec<(Target, String)>,
}

impl Listing {
    /// The listing once `selection` is chosen, whose own joints are
    /// `joints`. Choosing one of the listed joints keeps the listing: the
    /// joint stays under the part it was chosen under rather than moving
    /// to (or vanishing with) the selection. A joint pair is not a part, so
    /// choosing one keeps it wherever it was chosen from; a piece of
    /// hardware is, so it keeps the listing only when its row under the
    /// part was the one `clicked`.
    fn next(
        &self,
        selection: Option<&Selection>,
        joints: Vec<(Target, String)>,
        clicked: Option<Target>,
    ) -> Listing {
        let chosen_here = selection.is_some_and(|selection| {
            self.joints.iter().any(|(target, _)| {
                Selection::of_target(*target) == *selection
                    && (selection.item().is_none() || clicked == Some(*target))
            })
        });
        if chosen_here {
            self.clone()
        } else {
            Listing {
                under: selection.and_then(Selection::item),
                joints,
            }
        }
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
        listing: &state_ref.listing,
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
            .label(
                state_ref
                    .placeholder
                    .as_deref()
                    .unwrap_or("Nothing to show yet"),
            )
            .css_classes(["dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .margin_top(12)
            .margin_start(12)
            .margin_end(12)
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

fn sync_selection(list: &gtk::ListBox, state_cell: &Rc<RefCell<TreeState>>) {
    let mut state = state_cell.borrow_mut();
    let was = state.syncing;
    state.syncing = true;
    // A joint chosen under a part is highlighted there, even when what it
    // selects (a piece of hardware) has a row of its own too.
    let under_part = state.listing.under != state.selection.as_ref().and_then(Selection::item);
    let joint_row = state
        .selection
        .as_ref()
        .filter(|_| under_part)
        .and_then(|selection| {
            state.rows.iter().find_map(|(row, kind)| match kind {
                RowKind::Joint(target) if *selection == Selection::of_target(*target) => {
                    Some(row.clone())
                }
                RowKind::Joint(_)
                | RowKind::Entry { .. }
                | RowKind::Assembly(_)
                | RowKind::Source(_)
                | RowKind::Heading => None,
            })
        });
    let selected = joint_row.or_else(|| {
        state.selection.as_ref().and_then(|selection| {
            state.rows.iter().find_map(|(row, kind)| match kind {
                RowKind::Entry { items, .. }
                    if selection.item().is_some_and(|item| items.contains(&item)) =>
                {
                    Some(row.clone())
                }
                RowKind::Assembly(id) if *selection == Selection::Assembly(*id) => {
                    Some(row.clone())
                }
                RowKind::Source(location)
                    if matches!(selection, Selection::Location(l) if l == location) =>
                {
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
        })
    });
    // Selecting a row calls the handler above, which borrows the state.
    drop(state);
    match selected {
        Some(row) => list.select_row(Some(&row)),
        None => list.unselect_all(),
    }
    state_cell.borrow_mut().syncing = was;
}

/// What the rows show besides the tree itself.
struct Shown<'a> {
    selection: Option<&'a Selection>,
    marks: &'a Marks,
    listing: &'a Listing,
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
    // The part the joints are listed under: the selection, or the part a
    // chosen joint was listed under.
    let selected = shown.listing.under;
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
                joint_rows(&shown.listing.joints, depth, rows);
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
            joint_rows(&shown.listing.joints, depth, rows);
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
    let open = is_open(node, toggled, shown.selection, shown.listing.under);
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
/// holds the selection or the part whose joints are listed (`under`).
fn is_open(
    node: &AssemblyNode,
    toggled: &BTreeMap<AssemblyId, bool>,
    selection: Option<&Selection>,
    under: Option<stave_scene::Item>,
) -> bool {
    toggled.get(&node.id).copied().unwrap_or_else(|| {
        selection.is_some_and(|s| holds(node, s))
            || under.is_some_and(|item| holds(node, &Selection::of_item(item)))
    })
}

/// The assemblies whose rows show, in tree order.
fn open_assemblies(
    nodes: &[AssemblyNode],
    toggled: &BTreeMap<AssemblyId, bool>,
    selection: Option<&Selection>,
    under: Option<stave_scene::Item>,
) -> Vec<AssemblyId> {
    let mut open = Vec::new();
    let mut stack: Vec<&AssemblyNode> = nodes.iter().rev().collect();
    while let Some(node) = stack.pop() {
        if is_open(node, toggled, selection, under) {
            open.push(node.id);
            stack.extend(node.children.iter().rev());
        }
    }
    open
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

#[cfg(test)]
mod tests {
    use super::*;
    use stave_model::solved::{HardwareId, JointId, PartId};
    use stave_scene::Item;

    fn base() -> Item {
        Item::Part(PartId::from_usize(0))
    }

    fn dowel(pair: usize) -> Target {
        Target::Joint {
            joint: JointId::from_usize(3),
            pair,
        }
    }

    fn hinge() -> Target {
        Target::Hardware(HardwareId::from_usize(1))
    }

    /// `base` selected, with its joints listed under it.
    fn base_listing() -> Listing {
        Listing::default().next(
            Some(&Selection::of_item(base())),
            vec![
                (dowel(0), "side_l into base (dowel)".to_owned()),
                (dowel(1), "side_r into base (dowel)".to_owned()),
                (hinge(), "hinge on base".to_owned()),
            ],
            None,
        )
    }

    #[test]
    fn selecting_a_part_lists_its_joints_under_it() {
        let listing = base_listing();
        assert_eq!(listing.under, Some(base()));
        assert_eq!(listing.joints.len(), 3);
    }

    #[test]
    fn choosing_a_listed_joint_keeps_it_under_the_part() {
        let listing = base_listing();
        // A joint pair is not a part: the window lists no joints for it.
        let next = listing.next(
            Some(&Selection::of_target(dowel(1))),
            Vec::new(),
            Some(dowel(1)),
        );
        assert_eq!(next, listing);
        // Chosen from elsewhere (the Details panel) it stays put too.
        let next = listing.next(Some(&Selection::of_target(dowel(0))), Vec::new(), None);
        assert_eq!(next, listing);
    }

    #[test]
    fn hardware_clicked_under_the_part_stays_there() {
        let listing = base_listing();
        let hinge_joints = vec![(dowel(0), "elsewhere".to_owned())];
        let next = listing.next(
            Some(&Selection::of_target(hinge())),
            hinge_joints.clone(),
            Some(hinge()),
        );
        assert_eq!(next, listing);
        // Picked in the 3D view instead, it is its own selection.
        let next = listing.next(Some(&Selection::of_target(hinge())), hinge_joints, None);
        assert_eq!(next.under, Some(Item::Hardware(HardwareId::from_usize(1))));
    }

    #[test]
    fn choosing_something_else_moves_the_listing() {
        let listing = base_listing();
        let other = Item::Part(PartId::from_usize(5));
        let next = listing.next(Some(&Selection::of_item(other)), Vec::new(), None);
        assert_eq!(next.under, Some(other));
        assert!(next.joints.is_empty());
        assert_eq!(listing.next(None, Vec::new(), None), Listing::default());
        // A joint that is not listed does not keep it either.
        let next = listing.next(Some(&Selection::of_target(dowel(7))), Vec::new(), None);
        assert_eq!(next, Listing::default());
    }

    #[test]
    fn the_tree_selecting_a_row_itself_is_not_a_choice() {
        crate::ui::gtk_test::on_gtk_thread(tree_selects_rows_quietly);
    }

    /// Showing the selection selects its row with nothing borrowed, and
    /// the row handler hears that as the tree's own change, not a click;
    /// a row chosen by hand is still passed on.
    fn tree_selects_rows_quietly() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stool");
        let opened = stave_app::open(&folder).expect("the stool opens");
        let design = opened.design.as_ref().expect("the stool solves");
        let sidebar = TreeSidebar::new();
        let chosen = Rc::new(RefCell::new(Vec::new()));
        sidebar.connect_select({
            let chosen = Rc::clone(&chosen);
            move |selection| chosen.borrow_mut().push(selection)
        });
        sidebar.show_tree(stave_app::inspect::parts_tree(design));
        let part = design.parts.first().expect("a part");
        let selection = Selection::of_item(Item::Part(part.id));
        sidebar.show_selection(Some(&selection), Vec::new());
        assert!(
            sidebar.list.selected_row().is_some(),
            "the part's row is selected"
        );
        assert!(
            chosen.borrow().is_empty(),
            "the tree's own change is no choice"
        );
        // Showing it again leaves the rows as they are and syncs only.
        sidebar.show_selection(Some(&selection), Vec::new());
        assert!(chosen.borrow().is_empty());
        let other = sidebar
            .state
            .borrow()
            .rows
            .iter()
            .find(|(row, kind)| {
                matches!(kind, RowKind::Entry { .. })
                    && Some(row) != sidebar.list.selected_row().as_ref()
            })
            .map(|(row, _)| row.clone())
            .expect("another part's row");
        sidebar.list.select_row(Some(&other));
        assert_eq!(
            chosen.borrow().len(),
            1,
            "a row chosen by hand is passed on"
        );
    }
}
