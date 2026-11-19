//! The model tree, floating over the 3D view (V2C_Home.dc.html): the
//! carcass's parts, then each assembly with its motion, its parts and
//! hardware, each row with a dot for its worst problem
//! (`stave_app::problems::marks`), and under the selected part the joints
//! it is in (`stave_app::focus::targets_of`), which open focus mode. Rows
//! come from `stave_app::inspect::parts_tree`. With no parts it says why
//! (`stave_app::empty`), and for a library folder lists what the library
//! defines; choosing one of those opens its text.
//!
//! It is a real tree: a `gtk::ListView` over a `gtk::TreeListModel`, each
//! row a `gtk::TreeExpander`, so assemblies open and shut with their arrow,
//! Right and Left (or + and −) and a screen reader hears the nesting. The
//! rows live in `gio::ListStore`s that each new tree, mark or selection
//! updates in place (`sync_store`): only rows that changed are replaced,
//! so the scroll position, the keyboard's row and what is open survive a
//! check. Clicking a row, or Enter on it, chooses it; the arrow keys only
//! move through the list, so the camera and the editor stay put while the
//! keyboard reads it.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use stave_app::empty::{EmptyState, LibraryItem};
use stave_app::focus::Target;
use stave_app::inspect::{AssemblyNode, PartsTree, Swatch, SwatchShape, TreeEntry};
use stave_app::problems::{Mark, Marks};
use stave_app::session::{Selection, SourceLocation};
use stave_model::solved::AssemblyId;
use stave_scene::Item;

use super::icons;

/// Called with what a row stands for when it is chosen.
type SelectCallback = Rc<dyn Fn(Selection)>;
/// Called with where a library's item is written when it is chosen.
type SourceCallback = Rc<dyn Fn(SourceLocation)>;
/// Called with a joint to focus on when its row is chosen.
type FocusCallback = Rc<dyn Fn(Target)>;

/// What a row of the tree is, with everything it shows.
#[derive(Debug, Clone, PartialEq)]
enum Row {
    /// "CARCASS", "ASSEMBLIES" or a library's group; `first` at the top.
    Heading { text: String, first: bool },
    /// A line saying why there is nothing to show: `centred` for the
    /// placeholder, otherwise the empty state's note.
    Note { text: String, centred: bool },
    /// A part or piece of hardware, a group of them, or one member.
    Entry {
        label: String,
        detail: Option<String>,
        swatch: Swatch,
        mark: Option<Mark>,
        /// What choosing it selects: its first item.
        selection: Selection,
        /// Everything it stands for, so selecting any of them in the 3D
        /// view highlights it.
        items: Vec<Item>,
    },
    /// An assembly, with its parts and the assemblies inside it below.
    Assembly {
        id: AssemblyId,
        label: String,
        detail: Option<String>,
        mark: Option<Mark>,
    },
    /// A joint the selected part is in: opens focus mode.
    Joint { target: Target, title: String },
    /// Something a library defines, and where it is written.
    Library {
        name: String,
        summary: Option<String>,
        location: Option<SourceLocation>,
    },
}

/// What makes a row the same row from one tree to the next: rows with the
/// same key are updated in place rather than replaced.
#[derive(Debug, PartialEq, Eq)]
enum RowKey<'a> {
    Heading(&'a str),
    Note,
    Entry(&'a [Item]),
    Assembly(AssemblyId),
    Joint(Target),
    Library(&'a str),
}

impl Row {
    fn key(&self) -> RowKey<'_> {
        match self {
            Row::Heading { text, .. } => RowKey::Heading(text),
            Row::Note { .. } => RowKey::Note,
            Row::Entry { items, .. } => RowKey::Entry(items),
            Row::Assembly { id, .. } => RowKey::Assembly(*id),
            Row::Joint { target, .. } => RowKey::Joint(*target),
            Row::Library { name, .. } => RowKey::Library(name),
        }
    }

    /// Whether the row can be chosen (and so selected and focused).
    fn choosable(&self) -> bool {
        match self {
            Row::Entry { .. } | Row::Assembly { .. } | Row::Joint { .. } => true,
            Row::Library { location, .. } => location.is_some(),
            Row::Heading { .. } | Row::Note { .. } => false,
        }
    }
}

/// A row and, for an assembly, the rows inside it.
#[derive(Debug, Clone, PartialEq)]
struct Node {
    row: Row,
    children: Option<Vec<Node>>,
}

impl Node {
    fn leaf(row: Row) -> Node {
        Node {
            row,
            children: None,
        }
    }
}

/// What a store's item holds: its row, the store of the rows inside it,
/// and the list item showing it, if any, so a change shows at once.
struct Slot {
    row: Row,
    children: Option<gio::ListStore>,
    bound: Option<glib::WeakRef<gtk::ListItem>>,
}

type SlotRef = Rc<RefCell<Slot>>;

/// The slot a store's item holds.
fn slot(object: &glib::Object) -> Option<SlotRef> {
    let boxed = object.downcast_ref::<glib::BoxedAnyObject>()?;
    boxed
        .try_borrow::<SlotRef>()
        .ok()
        .map(|slot| Rc::clone(&slot))
}

/// The slot a tree row shows.
fn row_slot(tree_row: &gtk::TreeListRow) -> Option<SlotRef> {
    tree_row.item().as_ref().and_then(slot)
}

/// The Parts panel's tree.
pub struct TreeSidebar {
    /// The panel's content.
    pub root: gtk::Box,
    list: gtk::ListView,
    selection: gtk::SingleSelection,
    model: gtk::TreeListModel,
    /// The top-level rows.
    store: gio::ListStore,
    state: Rc<RefCell<TreeState>>,
}

#[derive(Default)]
struct TreeState {
    tree: PartsTree,
    selection: Option<Selection>,
    /// The latest check's marks, for the dots.
    marks: Rc<Marks>,
    /// The part whose joints are listed under its row, and those joints.
    listing: Listing,
    /// The joint row being chosen, while its callback runs: the selection
    /// it makes keeps the joints listed where it was clicked.
    clicked: Option<Target>,
    /// Why there are no parts, when there are none.
    empty: Option<EmptyState>,
    /// What an empty tree says instead of "Nothing to show yet" (a new
    /// project's envelope: where to click).
    placeholder: Option<String>,
    on_select: Option<SelectCallback>,
    on_source: Option<SourceCallback>,
    on_focus: Option<FocusCallback>,
    /// The item last selected to show the window's selection. While it
    /// stays the one, a new result leaves the list's selection alone, so
    /// the keyboard can be elsewhere in the list.
    shown: Option<glib::Object>,
    /// The first click of a double click chose the row; the activation
    /// the second one makes only opens or shuts an assembly.
    double_click: bool,
}

/// How a row is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    /// One click: chooses it.
    Click,
    /// Enter: chooses it, and opens or shuts an assembly.
    Activate,
    /// The second click of a double click: opens or shuts an assembly.
    SecondClick,
}

/// A key that walks the tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Right: opens a shut row, or goes to an open one's first child.
    Open,
    /// Left: shuts an open row, or goes to its parent.
    Shut,
    /// +: opens a shut row.
    Expand,
    /// −: shuts an open row.
    Collapse,
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
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let model = gtk::TreeListModel::new(store.clone(), false, false, |object| {
            slot(object)?
                .borrow()
                .children
                .clone()
                .map(|children| children.upcast())
        });
        let selection = gtk::SingleSelection::builder()
            .model(&model)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let state = Rc::new(RefCell::new(TreeState::default()));
        let list = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory(&state))
            .accessible_role(gtk::AccessibleRole::Tree)
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
            selection,
            model,
            store,
            state,
        };
        sidebar.connect();
        sidebar.refresh(false, false);
        sidebar
    }

    fn connect(&self) {
        // Enter, or a double click.
        let state = Rc::clone(&self.state);
        self.list.connect_activate(move |list, position| {
            let second = std::mem::take(&mut state.borrow_mut().double_click);
            let Some(tree_row) = list
                .model()
                .and_then(|model| model.item(position))
                .and_downcast::<gtk::TreeListRow>()
            else {
                return;
            };
            choose(
                &state,
                &tree_row,
                if second {
                    How::SecondClick
                } else {
                    How::Activate
                },
            );
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(|controller, key, _, modifiers| {
            use gtk::gdk::{Key, ModifierType};
            if modifiers.intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }
            let Some(list) = controller.widget().and_downcast::<gtk::ListView>() else {
                return glib::Propagation::Proceed;
            };
            // Right opens and Left shuts, mirrored when the text runs
            // right to left.
            let rtl = list.direction() == gtk::TextDirection::Rtl;
            let (right, left) = if rtl {
                (Step::Shut, Step::Open)
            } else {
                (Step::Open, Step::Shut)
            };
            let step = match key {
                Key::Right | Key::KP_Right => right,
                Key::Left | Key::KP_Left => left,
                Key::plus | Key::KP_Add => Step::Expand,
                Key::minus | Key::KP_Subtract => Step::Collapse,
                _ => return glib::Propagation::Proceed,
            };
            if walk(&list, step) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.list.add_controller(keys);
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

    /// Shows a new tree, keeping what is open; rows that did not change
    /// are left as they are.
    pub fn show_tree(&self, tree: PartsTree) {
        {
            let mut state = self.state.borrow_mut();
            if state.tree == tree && state.empty.is_none() {
                return;
            }
            state.tree = tree;
            state.empty = None;
        }
        self.refresh(false, false);
    }

    /// Shows why there are no parts; for a library, what it defines.
    pub fn show_empty(&self, empty: EmptyState) {
        {
            let mut state = self.state.borrow_mut();
            if state.empty.as_ref() == Some(&empty) && state.tree == PartsTree::default() {
                return;
            }
            state.tree = PartsTree::default();
            state.empty = Some(empty);
        }
        self.refresh(false, false);
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
            self.refresh(false, false);
        }
    }

    /// Takes the latest check's marks for the dots: only the rows whose
    /// mark changed are touched.
    pub fn show_marks(&self, marks: Rc<Marks>) {
        {
            let mut state = self.state.borrow_mut();
            if *state.marks == *marks {
                return;
            }
            state.marks = marks;
        }
        self.refresh(false, false);
    }

    /// Highlights the selection's row, opening the assemblies it is in,
    /// and lists the joints the selected part is in under it. Choosing one
    /// of those joints keeps them listed under the same part.
    pub fn show_selection(&self, selection: Option<&Selection>, joints: Vec<(Target, String)>) {
        let changed = {
            let mut state = self.state.borrow_mut();
            let listing = state.listing.next(selection, joints, state.clicked);
            let changed = state.selection.as_ref() != selection;
            state.selection = selection.cloned();
            state.listing = listing;
            changed
        };
        self.refresh(changed, changed);
    }

    /// Brings the rows up to date with the state. `reveal` opens the
    /// assemblies the selection is in; `force` selects its row even when
    /// it is the one selected last time.
    fn refresh(&self, reveal: bool, force: bool) {
        let nodes = nodes(&self.state.borrow());
        sync_store(&self.store, nodes);
        if reveal {
            self.reveal();
        }
        self.sync_selection(force);
    }

    /// Opens the assemblies the selection (or the part its joints are
    /// listed under) is in, so its row shows. The assembly that is the
    /// selection is left as it is: choosing it opens or shuts nothing.
    fn reveal(&self) {
        let open = {
            let state = self.state.borrow();
            let mut open = Vec::new();
            for node in &state.tree.assemblies {
                containing(
                    node,
                    state.selection.as_ref(),
                    state.listing.under,
                    &mut open,
                );
            }
            open
        };
        if open.is_empty() {
            return;
        }
        // Opening a row adds its children after it, so walking forward
        // reaches the assemblies inside it too.
        let mut position = 0;
        while let Some(tree_row) = self.model.row(position) {
            let id = row_slot(&tree_row).and_then(|slot| match slot.borrow().row {
                Row::Assembly { id, .. } => Some(id),
                Row::Heading { .. }
                | Row::Note { .. }
                | Row::Entry { .. }
                | Row::Joint { .. }
                | Row::Library { .. } => None,
            });
            if id.is_some_and(|id| open.contains(&id)) && !tree_row.is_expanded() {
                tree_row.set_expanded(true);
            }
            position += 1;
        }
    }

    /// Selects the selection's row, or none. When it is the row selected
    /// last time and `force` is off, the list's selection is left where
    /// the keyboard put it.
    fn sync_selection(&self, force: bool) {
        let target = self.selected_row();
        {
            let mut state = self.state.borrow_mut();
            let object = target.as_ref().map(|(_, object)| object.clone());
            if !force && state.shown == object {
                return;
            }
            state.shown = object;
        }
        self.selection
            .set_selected(target.map_or(gtk::INVALID_LIST_POSITION, |(position, _)| position));
    }

    /// The showing row that stands for the selection, and its item. A
    /// joint chosen under a part is that row, even when what it selects (a
    /// piece of hardware) has a row of its own too.
    fn selected_row(&self) -> Option<(u32, glib::Object)> {
        let state = self.state.borrow();
        let selection = state.selection.as_ref()?;
        let under_part = state.listing.under != selection.item();
        let mut first = None;
        let mut position = 0;
        while let Some(tree_row) = self.model.row(position) {
            if let Some(object) = tree_row.item()
                && let Some(slot) = slot(&object)
            {
                let matches = match &slot.borrow().row {
                    Row::Joint { target, .. } if *selection == Selection::of_target(*target) => {
                        if under_part {
                            return Some((position, object));
                        }
                        true
                    }
                    Row::Entry { items, .. } => {
                        selection.item().is_some_and(|item| items.contains(&item))
                    }
                    Row::Assembly { id, .. } => *selection == Selection::Assembly(*id),
                    Row::Library {
                        location: Some(location),
                        ..
                    } => matches!(selection, Selection::Location(l) if l == location),
                    Row::Joint { .. }
                    | Row::Library { location: None, .. }
                    | Row::Heading { .. }
                    | Row::Note { .. } => false,
                };
                if matches && first.is_none() {
                    first = Some((position, object));
                }
            }
            position += 1;
        }
        first
    }
}

/// The factory making each row: a tree expander round a box the row's
/// widgets are put in when it is bound.
fn factory(state: &Rc<RefCell<TreeState>>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let weak = Rc::downgrade(state);
    factory.connect_setup(move |_, object| {
        let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .build();
        let expander = gtk::TreeExpander::builder().child(&content).build();
        list_item.set_child(Some(&expander));
        // One click chooses the row, as it always has; the arrow (outside
        // `content`) only opens and shuts.
        let click = gtk::GestureClick::new();
        let pressed_state = weak.clone();
        let pressed_item = list_item.downgrade();
        click.connect_pressed(move |_, presses, _, _| {
            if presses > 1
                && let Some(state) = pressed_state.upgrade()
                && pressed_item
                    .upgrade()
                    .is_some_and(|item| item.is_activatable())
            {
                state.borrow_mut().double_click = true;
            }
        });
        let released_state = weak.clone();
        let released_item = list_item.downgrade();
        click.connect_released(move |_, presses, _, _| {
            if presses != 1 {
                return;
            }
            let (Some(state), Some(list_item)) =
                (released_state.upgrade(), released_item.upgrade())
            else {
                return;
            };
            state.borrow_mut().double_click = false;
            // Once the list has selected the clicked row itself: choosing
            // can change the rows, and this widget may then show another.
            if let Some(tree_row) = list_item.item().and_downcast::<gtk::TreeListRow>() {
                glib::idle_add_local_once(move || choose(&state, &tree_row, How::Click));
            }
        });
        content.add_controller(click);
    });
    factory.connect_bind(|_, object| {
        let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(tree_row) = list_item.item().and_downcast::<gtk::TreeListRow>() else {
            return;
        };
        if let Some(expander) = list_item.child().and_downcast::<gtk::TreeExpander>() {
            expander.set_list_row(Some(&tree_row));
        }
        let Some(slot) = row_slot(&tree_row) else {
            return;
        };
        slot.borrow_mut().bound = Some(list_item.downgrade());
        show_row(list_item, &slot.borrow().row);
    });
    factory.connect_unbind(|_, object| {
        let Some(list_item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let Some(slot) = list_item
            .item()
            .and_downcast::<gtk::TreeListRow>()
            .and_then(|tree_row| row_slot(&tree_row))
        {
            let mut slot = slot.borrow_mut();
            if slot
                .bound
                .as_ref()
                .and_then(glib::WeakRef::upgrade)
                .is_some_and(|bound| bound == *list_item)
            {
                slot.bound = None;
            }
        }
        if let Some(expander) = list_item.child().and_downcast::<gtk::TreeExpander>() {
            expander.set_list_row(None);
        }
    });
    factory
}

/// Chooses a row: selects what it stands for, focuses on its joint or
/// opens its text; Enter or a double click on an assembly also opens or
/// shuts it.
fn choose(state: &Rc<RefCell<TreeState>>, tree_row: &gtk::TreeListRow, how: How) {
    let Some(row) = row_slot(tree_row).map(|slot| slot.borrow().row.clone()) else {
        return;
    };
    match row {
        Row::Assembly { id, .. } => {
            if how != How::Click {
                tree_row.set_expanded(!tree_row.is_expanded());
            }
            if how != How::SecondClick {
                let callback = state.borrow().on_select.clone();
                if let Some(callback) = callback {
                    callback(Selection::Assembly(id));
                }
            }
        }
        _ if how == How::SecondClick => {}
        Row::Entry { selection, .. } => {
            let callback = state.borrow().on_select.clone();
            if let Some(callback) = callback {
                callback(selection);
            }
        }
        Row::Joint { target, .. } => {
            let callback = state.borrow().on_focus.clone();
            if let Some(callback) = callback {
                state.borrow_mut().clicked = Some(target);
                callback(target);
                state.borrow_mut().clicked = None;
            }
        }
        Row::Library {
            location: Some(location),
            ..
        } => {
            let callback = state.borrow().on_source.clone();
            if let Some(callback) = callback {
                callback(location);
            }
        }
        Row::Library { location: None, .. } | Row::Heading { .. } | Row::Note { .. } => {}
    }
}

/// Walks the tree from the selected row (the keyboard's): `true` when the
/// key did something.
fn walk(list: &gtk::ListView, step: Step) -> bool {
    let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>() else {
        return false;
    };
    let Some(tree_row) = selection.selected_item().and_downcast::<gtk::TreeListRow>() else {
        return false;
    };
    let go = |position: u32| {
        list.scroll_to(
            position,
            gtk::ListScrollFlags::FOCUS | gtk::ListScrollFlags::SELECT,
            None,
        );
        true
    };
    let has_children = tree_row
        .children()
        .is_some_and(|children| children.n_items() > 0);
    match step {
        Step::Open | Step::Expand if tree_row.is_expandable() && !tree_row.is_expanded() => {
            tree_row.set_expanded(true);
            true
        }
        Step::Open if tree_row.is_expanded() && has_children => {
            tree_row.position().checked_add(1).is_some_and(go)
        }
        Step::Shut | Step::Collapse if tree_row.is_expanded() => {
            tree_row.set_expanded(false);
            true
        }
        Step::Shut => tree_row
            .parent()
            .is_some_and(|parent| go(parent.position())),
        Step::Open | Step::Expand | Step::Collapse => false,
    }
}

/// Makes `store` hold `nodes`, keeping the items of rows that are still
/// there (the same key at the start or the end) and updating them in
/// place; only the changed run in between is replaced. A kept assembly
/// keeps its store, so what is open inside it stays open.
fn sync_store(store: &gio::ListStore, mut nodes: Vec<Node>) {
    let old: Vec<(glib::Object, SlotRef)> = (0..store.n_items())
        .filter_map(|position| store.item(position))
        .filter_map(|object| slot(&object).map(|slot| (object, slot)))
        .collect();
    let same = |slot: &SlotRef, node: &Node| {
        let slot = slot.borrow();
        slot.row.key() == node.row.key() && slot.children.is_some() == node.children.is_some()
    };
    let prefix = old
        .iter()
        .zip(&nodes)
        .take_while(|((_, slot), node)| same(slot, node))
        .count();
    let suffix = old
        .iter()
        .skip(prefix)
        .rev()
        .zip(nodes.iter().skip(prefix).rev())
        .take_while(|((_, slot), node)| same(slot, node))
        .count();
    let tail = nodes.split_off(nodes.len().saturating_sub(suffix));
    let middle = nodes.split_off(prefix);
    let kept = old
        .iter()
        .take(prefix)
        .zip(nodes)
        .chain(old.iter().skip(old.len().saturating_sub(suffix)).zip(tail));
    for ((_, slot), node) in kept {
        update(slot, node);
    }
    let removed = old.len().saturating_sub(prefix).saturating_sub(suffix);
    let added: Vec<glib::BoxedAnyObject> = middle.into_iter().map(new_item).collect();
    if (removed > 0 || !added.is_empty())
        && let (Ok(position), Ok(removed)) = (u32::try_from(prefix), u32::try_from(removed))
    {
        store.splice(position, removed, &added);
    }
}

/// Brings a kept item up to date: its row (shown again at once if it is
/// on screen) and the rows inside it.
fn update(slot: &SlotRef, node: Node) {
    let (bound, children) = {
        let mut slot = slot.borrow_mut();
        let bound = if slot.row == node.row {
            None
        } else {
            slot.row = node.row;
            slot.bound.as_ref().and_then(glib::WeakRef::upgrade)
        };
        (bound, slot.children.clone())
    };
    if let Some(list_item) = bound {
        show_row(&list_item, &slot.borrow().row);
    }
    if let (Some(store), Some(nodes)) = (children, node.children) {
        sync_store(&store, nodes);
    }
}

/// A new item for a row, with a store of the rows inside it.
fn new_item(node: Node) -> glib::BoxedAnyObject {
    let children = node.children.map(|nodes| {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        sync_store(&store, nodes);
        store
    });
    let slot: SlotRef = Rc::new(RefCell::new(Slot {
        row: node.row,
        children,
        bound: None,
    }));
    glib::BoxedAnyObject::new(slot)
}

/// Puts a row's widgets into its list item, replacing what was there.
fn show_row(list_item: &gtk::ListItem, row: &Row) {
    let Some(expander) = list_item.child().and_downcast::<gtk::TreeExpander>() else {
        return;
    };
    let Some(content) = expander.child().and_downcast::<gtk::Box>() else {
        return;
    };
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    content.set_css_classes(&[]);
    content.set_tooltip_text(None);
    content.set_orientation(gtk::Orientation::Horizontal);
    let choosable = row.choosable();
    list_item.set_selectable(choosable);
    list_item.set_activatable(choosable);
    list_item.set_focusable(choosable);
    // Rows inside an assembly line up with its name, past its arrow.
    let in_tree = matches!(
        row,
        Row::Entry { .. } | Row::Assembly { .. } | Row::Joint { .. }
    );
    expander.set_indent_for_icon(in_tree);
    expander.set_indent_for_depth(in_tree);
    if let Some(tree_row) = expander.list_row() {
        let level = i32::try_from(tree_row.depth())
            .unwrap_or(i32::MAX)
            .saturating_add(1);
        expander.update_property(&[gtk::accessible::Property::Level(level)]);
    }
    let text = |label: &str| {
        gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build()
    };
    let dim = |label: &str| {
        gtk::Label::builder()
            .label(label)
            .css_classes(["stave-detail"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build()
    };
    let (name, description) = match row {
        Row::Heading {
            text: heading,
            first,
        } => {
            content.add_css_class("stave-heading");
            if *first {
                content.add_css_class("first");
            }
            content.append(&gtk::Label::builder().label(heading).xalign(0.0).build());
            (heading.clone(), String::new())
        }
        Row::Note {
            text: note,
            centred,
        } => {
            let label = gtk::Label::builder()
                .label(note)
                .wrap(true)
                .hexpand(true)
                .css_classes(["dim-label"])
                .build();
            if *centred {
                label.set_justify(gtk::Justification::Center);
                label.set_margin_top(12);
                label.set_margin_start(12);
                label.set_margin_end(12);
            } else {
                label.set_xalign(0.0);
                content.add_css_class("stave-sidebar-note");
            }
            content.append(&label);
            (note.clone(), String::new())
        }
        Row::Entry {
            label,
            detail,
            swatch,
            mark,
            ..
        } => {
            content.add_css_class("stave-entry");
            content.append(&icons::swatch(
                rgba(*swatch),
                swatch.shape == SwatchShape::Round,
                10,
            ));
            content.append(&text(label));
            if let Some(detail) = detail {
                content.append(&dim(detail));
            }
            content.append(&super::style::mark_dot(*mark));
            (label.clone(), described(detail.as_deref(), *mark))
        }
        Row::Assembly {
            label,
            detail,
            mark,
            ..
        } => {
            content.add_css_class("stave-assembly");
            content.append(&text(label));
            if let Some(detail) = detail {
                content.append(&dim(detail));
            }
            content.append(&super::style::mark_dot(*mark));
            (label.clone(), described(detail.as_deref(), *mark))
        }
        Row::Joint { target, title } => {
            content.add_css_class("stave-joint");
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
            content.append(&text(title));
            let name = format!("Focus on {title}");
            content.set_tooltip_text(Some(&name));
            (name, String::new())
        }
        Row::Library {
            name,
            summary,
            location,
        } => {
            content.set_orientation(gtk::Orientation::Vertical);
            content.add_css_class("stave-entry");
            content.add_css_class("stave-library-entry");
            content.append(&text(name));
            if let Some(summary) = summary {
                let label = text(summary);
                label.add_css_class("stave-detail");
                content.append(&label);
            }
            if location.is_some() {
                content.set_tooltip_text(Some(summary.as_deref().unwrap_or("Open its text")));
            }
            (name.clone(), summary.clone().unwrap_or_default())
        }
    };
    list_item.set_accessible_label(&name);
    list_item.set_accessible_description(&description);
}

/// What a screen reader hears after a row's name: its dim text and its
/// mark in words, since the dot is only a colour.
fn described(detail: Option<&str>, mark: Option<Mark>) -> String {
    let mark = mark.map(|mark| match mark {
        Mark::Error => "has an error",
        Mark::Warning => "has a warning",
        Mark::Involved => "involved in a problem",
    });
    [detail, mark]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
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

/// What the rows show besides the tree itself.
struct Shown<'a> {
    marks: &'a Marks,
    listing: &'a Listing,
}

/// The rows the state calls for, in order.
fn nodes(state: &TreeState) -> Vec<Node> {
    let tree = &state.tree;
    let shown = Shown {
        marks: &state.marks,
        listing: &state.listing,
    };
    let mut rows = Vec::new();
    if !tree.carcass.is_empty() {
        rows.push(heading("CARCASS", true));
        for entry in &tree.carcass {
            entry_nodes(entry, &shown, &mut rows);
        }
    }
    if !tree.assemblies.is_empty() {
        rows.push(heading("ASSEMBLIES", tree.carcass.is_empty()));
        for node in &tree.assemblies {
            rows.push(assembly_node(node, &shown));
        }
    }
    if rows.is_empty()
        && let Some(empty) = &state.empty
    {
        empty_nodes(empty, &mut rows);
    }
    if rows.is_empty() {
        rows.push(Node::leaf(Row::Note {
            text: state
                .placeholder
                .as_deref()
                .unwrap_or("Nothing to show yet")
                .to_owned(),
            centred: true,
        }));
    }
    rows
}

/// The rows that say why there are no parts: a line, then for a library
/// what it defines, by group.
fn empty_nodes(empty: &EmptyState, rows: &mut Vec<Node>) {
    rows.push(Node::leaf(Row::Note {
        text: empty.sidebar().to_owned(),
        centred: false,
    }));
    let Some(library) = &empty.library else {
        return;
    };
    for (group, items) in &library.groups {
        if items.is_empty() {
            continue;
        }
        rows.push(heading(group.heading(), false));
        rows.extend(items.iter().map(library_node));
    }
}

/// A row for something a library defines: its name and the first line of
/// its doc comment.
fn library_node(item: &LibraryItem) -> Node {
    Node::leaf(Row::Library {
        name: item.name.clone(),
        summary: item.summary.clone(),
        location: item.location.clone(),
    })
}

fn heading(text: &str, first: bool) -> Node {
    Node::leaf(Row::Heading {
        text: text.to_owned(),
        first,
    })
}

/// The rows of an entry: one, or one per member when the selection is
/// one of its members (so each can be chosen), with the joints listed
/// after the part they are under.
fn entry_nodes(entry: &TreeEntry, shown: &Shown<'_>, rows: &mut Vec<Node>) {
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
            rows.push(entry_node(
                entry,
                label,
                detail.as_deref(),
                vec![*item],
                shown.marks.item(*item),
            ));
            if selected == Some(*item) {
                joint_nodes(&shown.listing.joints, rows);
            }
        }
    } else {
        rows.push(entry_node(
            entry,
            &entry.label,
            entry.detail.as_deref(),
            entry.items.iter().copied().collect(),
            shown.marks.entry(entry),
        ));
        if selected.is_some_and(|item| entry.items.iter().any(|i| *i == item)) {
            joint_nodes(&shown.listing.joints, rows);
        }
    }
}

/// The joints the selected part is in, after its row; each opens focus
/// mode.
fn joint_nodes(joints: &[(Target, String)], rows: &mut Vec<Node>) {
    rows.extend(joints.iter().map(|(target, title)| {
        Node::leaf(Row::Joint {
            target: *target,
            title: title.clone(),
        })
    }));
}

fn entry_node(
    entry: &TreeEntry,
    label: &str,
    detail: Option<&str>,
    items: Vec<Item>,
    mark: Option<Mark>,
) -> Node {
    let first = items.first().copied().unwrap_or(*entry.items.first());
    Node::leaf(Row::Entry {
        label: label.to_owned(),
        detail: detail.map(str::to_owned),
        swatch: entry.swatch,
        mark,
        selection: Selection::of_item(first),
        items,
    })
}

/// An assembly's row, with its parts and the assemblies inside it.
fn assembly_node(node: &AssemblyNode, shown: &Shown<'_>) -> Node {
    let mut children = Vec::new();
    for entry in &node.entries {
        entry_nodes(entry, shown, &mut children);
    }
    children.extend(
        node.children
            .iter()
            .map(|child| assembly_node(child, shown)),
    );
    Node {
        row: Row::Assembly {
            id: node.id,
            label: node.label.clone(),
            detail: node.detail.clone(),
            mark: shown.marks.assembly(node.id),
        },
        children: Some(children),
    }
}

/// Collects into `open` the assemblies at or below `node` that the
/// selection's row (or the part `under`) is inside.
fn containing(
    node: &AssemblyNode,
    selection: Option<&Selection>,
    under: Option<Item>,
    open: &mut Vec<AssemblyId>,
) {
    if inside(node, selection, under) {
        open.push(node.id);
        for child in &node.children {
            containing(child, selection, under, open);
        }
    }
}

/// Whether the selection's row (or the part `under`) is among the rows
/// inside an assembly, at any depth.
fn inside(node: &AssemblyNode, selection: Option<&Selection>, under: Option<Item>) -> bool {
    let item = selection.and_then(Selection::item);
    node.entries.iter().any(|entry| {
        entry
            .items
            .iter()
            .any(|i| Some(*i) == item || Some(*i) == under)
    }) || node.children.iter().any(|child| {
        selection == Some(&Selection::Assembly(child.id)) || inside(child, selection, under)
    })
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

    /// The rows showing now, in order.
    fn shown_rows(sidebar: &TreeSidebar) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut position = 0;
        while let Some(tree_row) = sidebar.model.row(position) {
            if let Some(slot) = row_slot(&tree_row) {
                rows.push(slot.borrow().row.clone());
            }
            position += 1;
        }
        rows
    }

    /// The items of the top-level rows.
    fn top_items(sidebar: &TreeSidebar) -> Vec<glib::Object> {
        (0..sidebar.store.n_items())
            .filter_map(|position| sidebar.store.item(position))
            .collect()
    }

    fn sidebar_for(example: &str) -> (TreeSidebar, stave_app::Opened) {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(example);
        let opened = stave_app::open(&folder).expect("the example opens");
        let sidebar = TreeSidebar::new();
        let design = opened.design.as_ref().expect("the example solves");
        sidebar.show_tree(stave_app::inspect::parts_tree(design));
        (sidebar, opened)
    }

    #[test]
    fn the_tree_selecting_a_row_itself_is_not_a_choice() {
        crate::ui::gtk_test::on_gtk_thread(tree_selects_rows_quietly);
    }

    /// Showing the selection selects its row, and that is no choice; nor
    /// is moving the list's selection, as the arrow keys do. Choosing a
    /// row (Enter, a click) is passed on.
    fn tree_selects_rows_quietly() {
        let (sidebar, opened) = sidebar_for("stool");
        let design = opened.design.as_ref().expect("the stool solves");
        let chosen = Rc::new(RefCell::new(Vec::new()));
        sidebar.connect_select({
            let chosen = Rc::clone(&chosen);
            move |selection| chosen.borrow_mut().push(selection)
        });
        let part = design.parts.first().expect("a part");
        let selection = Selection::of_item(Item::Part(part.id));
        sidebar.show_selection(Some(&selection), Vec::new());
        let selected = sidebar.selection.selected();
        assert_ne!(
            selected,
            gtk::INVALID_LIST_POSITION,
            "the part's row is selected"
        );
        assert!(
            chosen.borrow().is_empty(),
            "the tree's own change is no choice"
        );
        sidebar.show_selection(Some(&selection), Vec::new());
        assert!(chosen.borrow().is_empty());
        let other = shown_rows(&sidebar)
            .iter()
            .enumerate()
            .position(|(position, row)| {
                matches!(row, Row::Entry { .. }) && u32::try_from(position).ok() != Some(selected)
            })
            .and_then(|position| u32::try_from(position).ok())
            .expect("another part's row");
        // The arrow keys move the list's selection: nothing is chosen.
        sidebar.selection.set_selected(other);
        assert!(
            chosen.borrow().is_empty(),
            "walking the list chooses nothing"
        );
        // A new result with the same selection leaves the keyboard's row.
        sidebar.show_selection(Some(&selection), Vec::new());
        assert_eq!(sidebar.selection.selected(), other);
        // Enter chooses it.
        sidebar.list.emit_by_name::<()>("activate", &[&other]);
        assert_eq!(chosen.borrow().len(), 1, "a chosen row is passed on");
    }

    #[test]
    fn a_new_result_updates_rows_in_place() {
        crate::ui::gtk_test::on_gtk_thread(rows_update_in_place);
    }

    /// The same tree again, or new marks, keep every row's item; a mark
    /// changes only that row.
    fn rows_update_in_place() {
        let (sidebar, opened) = sidebar_for("stool");
        let design = opened.design.as_ref().expect("the stool solves");
        let before = top_items(&sidebar);
        sidebar.show_tree(stave_app::inspect::parts_tree(design));
        sidebar.show_marks(Rc::default());
        assert_eq!(top_items(&sidebar), before, "nothing was rebuilt");

        // Fake a check that marks one part with an error: only its row's
        // look changes, and it is still the same item.
        let first_entry = shown_rows(&sidebar)
            .into_iter()
            .position(|row| matches!(row, Row::Entry { .. }))
            .expect("an entry");
        let mut nodes = nodes(&sidebar.state.borrow());
        if let Some(Node {
            row: Row::Entry { mark, .. },
            ..
        }) = nodes.get_mut(first_entry)
        {
            *mark = Some(Mark::Error);
        }
        sync_store(&sidebar.store, nodes);
        assert_eq!(top_items(&sidebar), before);
        assert!(matches!(
            shown_rows(&sidebar).get(first_entry),
            Some(Row::Entry {
                mark: Some(Mark::Error),
                ..
            })
        ));
    }

    #[test]
    fn only_the_changed_run_of_rows_is_replaced() {
        crate::ui::gtk_test::on_gtk_thread(changed_run_is_replaced);
    }

    fn changed_run_is_replaced() {
        let note = |text: &str| {
            Node::leaf(Row::Heading {
                text: text.to_owned(),
                first: false,
            })
        };
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        sync_store(&store, vec![note("A"), note("B"), note("C")]);
        let items: Vec<glib::Object> = (0..3).filter_map(|p| store.item(p)).collect();
        sync_store(&store, vec![note("A"), note("X"), note("Y"), note("C")]);
        assert_eq!(store.n_items(), 4);
        assert_eq!(store.item(0), items.first().cloned(), "A is kept");
        assert_eq!(store.item(3), items.get(2).cloned(), "C is kept");
        assert_ne!(store.item(1), items.get(1).cloned(), "B is replaced");
    }

    #[test]
    fn right_and_left_open_shut_and_walk_the_tree() {
        crate::ui::gtk_test::on_gtk_thread(keys_walk_the_tree);
    }

    /// Right opens an assembly, then goes into it; Left goes back to it,
    /// then shuts it. None of it chooses anything.
    fn keys_walk_the_tree() {
        let (sidebar, _opened) = sidebar_for("box");
        let chosen = Rc::new(RefCell::new(0));
        sidebar.connect_select({
            let chosen = Rc::clone(&chosen);
            move |_| *chosen.borrow_mut() += 1
        });
        let assembly = shown_rows(&sidebar)
            .iter()
            .position(|row| matches!(row, Row::Assembly { .. }))
            .and_then(|position| u32::try_from(position).ok())
            .expect("the box has an assembly");
        let tree_row = sidebar.model.row(assembly).expect("its row");
        assert!(!tree_row.is_expanded(), "assemblies start shut");
        sidebar.selection.set_selected(assembly);
        assert!(walk(&sidebar.list, Step::Open));
        assert!(tree_row.is_expanded(), "Right opens it");
        assert!(walk(&sidebar.list, Step::Open));
        assert_eq!(
            sidebar.selection.selected(),
            assembly + 1,
            "Right again goes to its first row"
        );
        assert!(walk(&sidebar.list, Step::Shut));
        assert_eq!(
            sidebar.selection.selected(),
            assembly,
            "Left goes back to the assembly"
        );
        assert!(walk(&sidebar.list, Step::Shut));
        assert!(!tree_row.is_expanded(), "Left again shuts it");
        assert!(walk(&sidebar.list, Step::Expand));
        assert!(walk(&sidebar.list, Step::Collapse));
        assert_eq!(*chosen.borrow(), 0, "walking chooses nothing");
    }

    #[test]
    fn choosing_a_part_inside_an_assembly_opens_it() {
        crate::ui::gtk_test::on_gtk_thread(selection_reveals_its_row);
    }

    fn selection_reveals_its_row() {
        let (sidebar, opened) = sidebar_for("box");
        let design = opened.design.as_ref().expect("the box solves");
        let tree = stave_app::inspect::parts_tree(design);
        let inner = tree
            .assemblies
            .iter()
            .flat_map(|node| node.entries.iter())
            .map(|entry| *entry.items.first())
            .next()
            .expect("a part inside an assembly");
        sidebar.show_selection(Some(&Selection::of_item(inner)), Vec::new());
        let selected = sidebar.selection.selected();
        assert_ne!(selected, gtk::INVALID_LIST_POSITION, "its row shows");
        let row = sidebar.model.row(selected).expect("the selected row");
        assert_eq!(row.depth(), 1, "inside its assembly");
    }
}
