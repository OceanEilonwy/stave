//! The Parts panel ("joint chips"): a flat list grouped by assembly. Each
//! group has a slim uppercase heading (CARCASS, DOOR_L) with a grey note
//! on the right ("12 parts · 6 joints", or how it moves: "110° on side_l"
//! after a turn icon). Each part row (24 px) has its colour swatch, its
//! name, small joint-kind chips (an icon per kind, with a count when there
//! is more than one; the tooltip names the kind) and a dot for its worst
//! problem (`stave_app::problems::marks`). Assemblies that are a mirror
//! image or one of a set fold into one row each (`door_r`, "mirror of
//! door_l"; `drawer[bottom, top]`) with their chips; activating a folded
//! row opens it and a heading folds it again. The selected part expands
//! into "joined to" rows: an icon, the joint kind in words ("groove, no
//! glue", "euro hinge ×2, back face", "turns on") and the partner names
//! as links (monospace light blue on a faint blue tint) that select the
//! partner when clicked and light it in the 3D view while hovered; the
//! partners' own rows get a light-blue left edge and a faint tint.
//! Activating a joined-to row opens focus mode on that joint or piece of
//! hardware. Hardware and a joint's pieces (pins) have no rows of their
//! own: choosing one (in the 3D view, say) expands the part it is on and
//! marks its joined-to row.
//!
//! Narrow panels show less (`stave_app::part_joints::Density`): below
//! 220 px the counts leave the chips (they stay in the tooltips), the
//! heading notes drop (the motion icon stays) and joined-to rows keep only
//! their icon and partners; below 160 px each row's chips fold into one
//! badge (a joint glyph and how many kinds, the kinds in its tooltip).
//!
//! Rows come from `stave_app::inspect::parts_tree` and
//! `stave_app::part_joints::PartJoints`; activating one asks the window to
//! select it. With no parts it says why (`stave_app::empty`), and for a
//! library folder lists what the library defines; activating one of those
//! opens its text.
//!
//! For the keyboard and screen readers the list is a tree (its role
//! `Tree`, each row a `TreeItem` with its level: headings and folded rows
//! 1, part rows 2, joined-to rows 3; a heading expanded, a folded row
//! not; a row's dim text and its mark in words as its description). The
//! arrow keys only move through the rows: a click or Enter chooses one.
//! Right (Left, right to left) or + opens a folded row and, on an open
//! heading, goes to its first row; Left or − folds a heading and, on a
//! part row, goes to its heading (on a joined-to row, to its part). A new
//! tree, new marks or a new selection update the rows in place: rows that
//! read the same are kept and only the run that differs is replaced, so
//! the scroll position and the row with the keyboard survive a check.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use adw::prelude::*;
use stave_app::empty::{EmptyState, LibraryItem};
use stave_app::focus::Target;
use stave_app::inspect::{AssemblyNode, PartsTree, Swatch, SwatchShape, TreeEntry};
use stave_app::part_joints::{
    self, Block, Chip, Density, Glyph, JoinedTo, Motion, PartJoints, Partner,
};
use stave_app::problems::{Mark, Marks};
use stave_app::session::{Selection, SourceLocation};
use stave_model::solved::AssemblyId;
use stave_scene::Item;

use super::icons::{self, Paint, PathIcon, Shape};

/// Called with what a row stands for when it is activated.
type SelectCallback = Rc<dyn Fn(Selection)>;
/// Called with where a library's item is written when it is activated.
type SourceCallback = Rc<dyn Fn(SourceLocation)>;
/// Called with a joint to focus on when its row is activated.
type FocusCallback = Rc<dyn Fn(Target)>;
/// Called with what the pointer is over (empty when it leaves).
type HoverCallback = Rc<dyn Fn(Vec<Item>)>;

/// The panel's own look: sizes, the chips' colours (light, and dark under
/// the window's `stave-dark`) and the partner links.
const CSS: &str = r#"
.stave-parts { background: transparent; font-size: 12.5px; }
.stave-parts > row { padding: 0; min-height: 0; margin: 0; border-radius: 0; }
.stave-parts > row:hover { background: alpha(currentColor, 0.06); }
.stave-parts > row:selected { background: @accent_bg_color; color: @accent_fg_color; }
.stave-parts > row.partner { box-shadow: inset 2px 0 0 #1c71d8; background: alpha(#3584e4, 0.08); }
.stave-dark .stave-parts > row.partner { box-shadow: inset 2px 0 0 #99c1f1; }
.stave-parts-heading { min-height: 22px; padding: 0 8px; margin-top: 4px; border-spacing: 4px; font-size: 10.5px; font-weight: 700; letter-spacing: 0.04em; color: alpha(currentColor, 0.62); }
.stave-parts-heading .note { font-weight: 500; letter-spacing: 0; }
.stave-parts-entry { min-height: 24px; padding: 0 8px; border-spacing: 7px; }
.stave-parts-entry .detail { font-size: 11.5px; color: alpha(currentColor, 0.62); }
.stave-parts-folded { min-height: 24px; padding: 0 8px 0 4px; border-spacing: 5px; }
.stave-parts-joined { min-height: 22px; padding: 0 8px 0 26px; border-spacing: 6px; font-size: 11.5px; background: alpha(#3584e4, 0.12); }
.stave-parts-joined .words { color: alpha(currentColor, 0.66); }
.stave-parts.narrow .stave-parts-joined { padding-left: 14px; }
.stave-chips { border-spacing: 3px; }
.stave-chip { min-height: 18px; padding: 0 4px; border-radius: 4px; border-spacing: 2px; background: alpha(currentColor, 0.06); }
.stave-chip label { font-size: 10.5px; color: alpha(currentColor, 0.72); }
.stave-glyph-wood { color: #9c6e00; }
.stave-glyph-cut { color: #1c71d8; }
.stave-glyph-frame { color: #9141ac; }
.stave-glyph-hardware { color: #c64600; }
.stave-glyph-plain { color: alpha(currentColor, 0.7); }
.stave-glyph-motion { color: #1c71d8; }
.stave-dark .stave-glyph-wood { color: #f8e45c; }
.stave-dark .stave-glyph-cut { color: #78aeed; }
.stave-dark .stave-glyph-frame { color: #dc8add; }
.stave-dark .stave-glyph-hardware { color: #ffbe6f; }
.stave-dark .stave-glyph-motion { color: #99c1f1; }
.stave-parts > row:selected staveicon { color: @accent_fg_color; }
.stave-parts > row:selected .stave-chip { background: alpha(@accent_fg_color, 0.16); }
.stave-parts > row:selected .stave-chip label, .stave-parts > row:selected .detail { color: alpha(@accent_fg_color, 0.9); }
button.stave-partner { min-height: 18px; min-width: 0; padding: 0 5px; border-radius: 4px; font-family: monospace; font-size: 11.5px; font-weight: 400; color: #1c71d8; background: alpha(#3584e4, 0.10); }
button.stave-partner:hover { background: alpha(#3584e4, 0.24); }
.stave-dark button.stave-partner { color: #99c1f1; background: alpha(#3584e4, 0.16); }
.stave-dark button.stave-partner:hover { background: alpha(#3584e4, 0.30); }
.stave-partner-text { font-family: monospace; font-size: 11.5px; color: alpha(currentColor, 0.75); padding: 0 2px; }
"#;

thread_local! {
    /// Whether [`CSS`] is installed on the default display.
    static STYLED: Cell<bool> = const { Cell::new(false) };
}

fn install_css() {
    if STYLED.with(Cell::get) {
        return;
    }
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    STYLED.with(|styled| styled.set(true));
}

/// A stroked 16-unit icon.
const fn stroke(paths: &'static [&'static str]) -> Shape {
    Shape {
        view_box: 16.0,
        paint: Paint::Stroke(1.5),
        paths,
        tilted: false,
    }
}

/// The shape and colour class of a glyph.
fn glyph_shape(glyph: Glyph) -> (Shape, &'static str) {
    match glyph {
        Glyph::Pins => (
            stroke(&[
                "M7.2 8a2.2 2.2 0 1 1-4.4 0a2.2 2.2 0 1 1 4.4 0z",
                "M13.2 8a2.2 2.2 0 1 1-4.4 0a2.2 2.2 0 1 1 4.4 0z",
            ]),
            "stave-glyph-wood",
        ),
        Glyph::Tenon => (stroke(&["M3 3h10v5H3zM6 8v5h4V8"]), "stave-glyph-wood"),
        Glyph::Frame => (stroke(&["M1.5 5h5v2.5h3V5h5v6h-13z"]), "stave-glyph-frame"),
        Glyph::Dovetail => (
            stroke(&["M2 13h12M4.5 13L3 4h10l-1.5 9"]),
            "stave-glyph-frame",
        ),
        Glyph::Shaped => (stroke(&["M2 12c2-6 4-6 6-2s4 2 6-5"]), "stave-glyph-frame"),
        Glyph::Slot => (stroke(&["M1.5 6h13M1.5 10h13M5 8h6"]), "stave-glyph-cut"),
        Glyph::Channel => (stroke(&["M1.5 5h4v6h5V5h4"]), "stave-glyph-cut"),
        Glyph::Hole => (
            stroke(&[
                "M4.5 2.5h7a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2h-7a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2z",
                "M10.5 8a2.5 2.5 0 1 1-5 0a2.5 2.5 0 1 1 5 0z",
            ]),
            "stave-glyph-cut",
        ),
        Glyph::Butt => (stroke(&["M2 5h5v6H2zM9 5h5v6H9z"]), "stave-glyph-plain"),
        Glyph::Hinge => (
            stroke(&["M8 8a3 3 0 1 1-6 0a3 3 0 1 1 6 0z", "M8 8h6.5M12 5.5v5"]),
            "stave-glyph-hardware",
        ),
        Glyph::Slide => (
            stroke(&["M1.5 6h13M1.5 10h13M11 3.5L14 6l-3 2.5"]),
            "stave-glyph-hardware",
        ),
        Glyph::Pull => (
            stroke(&["M11.5 6a3.5 3.5 0 1 1-7 0a3.5 3.5 0 1 1 7 0z", "M8 9.5v5"]),
            "stave-glyph-hardware",
        ),
        Glyph::Mirror => (
            stroke(&["M8 1.5v13M2 4l4 4-4 4zM14 4l-4 4 4 4z"]),
            "stave-glyph-plain",
        ),
        Glyph::Turn => (TURNS, "stave-glyph-motion"),
        Glyph::Travel => (SLIDES, "stave-glyph-motion"),
    }
}

/// The badge's glyph: two pieces joined.
const JOINTS: Shape = stroke(&["M2 5h5v6H2zM9 5h5v6H9zM7 8h2"]);
/// A turn: a circling arrow.
const TURNS: Shape = stroke(&["M3 12a6 6 0 0 1 9-8.5M12 1v3h-3"]);
/// A slide: an arrow out along rails.
const SLIDES: Shape = stroke(&["M1.5 6h13M1.5 10h13M11 3.5L14 6l-3 2.5"]);

/// A glyph's icon, `size` pixels, with its tooltip.
fn glyph_icon(glyph: Glyph, size: i32, tooltip: &str) -> PathIcon {
    let (shape, class) = glyph_shape(glyph);
    let icon = PathIcon::new(shape, size);
    icon.add_css_class(class);
    icon.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    icon
}

/// What a row of the list is.
#[derive(Debug, Clone)]
enum RowKind {
    /// Not activatable: a note, the carcass's heading.
    Note,
    /// An assembly's heading: folds it.
    Heading(AssemblyId),
    /// A part row.
    Entry {
        /// What activating it selects: its first item.
        selection: Selection,
        /// Everything it stands for, so selecting any of them in the 3D
        /// view highlights it.
        items: Vec<Item>,
    },
    /// Folded assemblies: opens them.
    Folded(Vec<AssemblyId>),
    /// A joined-to row under the selected part.
    Joined {
        /// Focus mode on it.
        focus: Option<Target>,
        /// What it stands for.
        stands_for: Vec<Selection>,
        /// What activating it selects when there is nothing to focus on.
        selection: Option<Selection>,
    },
    /// Something a library defines: where it is written.
    Source(SourceLocation),
}

/// The Parts panel.
pub struct TreeSidebar {
    /// The panel's content.
    pub root: gtk::Box,
    list: gtk::ListBox,
    state: Rc<RefCell<TreeState>>,
}

#[derive(Default)]
struct TreeState {
    tree: PartsTree,
    joints: PartJoints,
    rows: Vec<(gtk::ListBoxRow, RowKind)>,
    /// What each row reads (`print`), to keep the rows a new fill makes
    /// the same.
    prints: Vec<String>,
    /// The selection the list last showed.
    synced: Option<Selection>,
    /// Assemblies the user opened (`true`) or folded (`false`); the
    /// others are as `PartJoints::folded_by_default` says, and open when
    /// they hold the selection.
    toggled: BTreeMap<AssemblyId, bool>,
    selection: Option<Selection>,
    /// The latest check's marks, for the dots.
    marks: Rc<Marks>,
    /// The part whose joined-to rows show under it.
    listing: Listing,
    /// The joined-to row being activated (or whose partner is clicked),
    /// while its callback runs: the selection it makes keeps the rows
    /// listed where it was clicked.
    clicked: Option<Selection>,
    /// How much the rows show at the panel's width.
    density: Option<Density>,
    /// Why there are no parts, when there are none.
    empty: Option<EmptyState>,
    /// What an empty tree says instead of "Nothing to show yet" (a new
    /// project's envelope: where to click).
    placeholder: Option<String>,
    on_select: Option<SelectCallback>,
    on_source: Option<SourceCallback>,
    on_focus: Option<FocusCallback>,
    on_hover: Option<HoverCallback>,
}

impl TreeState {
    fn density(&self) -> Density {
        self.density.unwrap_or(Density::Full)
    }

    /// Whether an assembly shows as a heading with its rows.
    fn is_open(&self, node: &AssemblyNode) -> bool {
        is_open(
            node,
            &self.joints,
            &self.toggled,
            self.selection.as_ref(),
            self.listing.under,
        )
    }
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
    /// An empty panel.
    pub fn new() -> TreeSidebar {
        install_css();
        let list = gtk::ListBox::builder()
            .accessible_role(gtk::AccessibleRole::Tree)
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["stave-parts"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Parts")]);
        // `External`: the panel may be narrower than its rows ask, so the
        // width watcher below sees the real width and the rows show less.
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vexpand(true)
            .child(&list)
            .build();
        // Watches the panel's width without taking part in its size.
        let watcher = gtk::DrawingArea::builder()
            .can_target(false)
            .can_focus(false)
            .build();
        let overlay = gtk::Overlay::builder().child(&scrolled).build();
        overlay.add_overlay(&watcher);
        overlay.set_measure_overlay(&watcher, false);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-model"])
            .build();
        root.append(&overlay);
        let sidebar = TreeSidebar {
            root,
            list,
            state: Rc::new(RefCell::new(TreeState::default())),
        };
        sidebar.connect();
        let state = sidebar.state.clone();
        let list = sidebar.list.clone();
        watcher.connect_resize(move |_, width, _| {
            let density = Density::for_width(width);
            let changed = {
                let Ok(mut state) = state.try_borrow_mut() else {
                    return;
                };
                let changed = state.density != Some(density);
                state.density = Some(density);
                changed
            };
            if changed {
                // Not while the panel is being laid out.
                let state = state.clone();
                let list = list.clone();
                gtk::glib::idle_add_local_once(move || fill(&list, &state));
            }
        });
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
                Some(RowKind::Heading(id)) => {
                    {
                        let mut state = state.borrow_mut();
                        let open = find(&state.tree.assemblies, id)
                            .is_some_and(|node| state.is_open(node));
                        state.toggled.insert(id, !open);
                    }
                    fill(list, &state);
                    focus_assembly(list, &state, id);
                    let callback = state.borrow().on_select.clone();
                    if let Some(callback) = callback {
                        callback(Selection::Assembly(id));
                    }
                }
                Some(RowKind::Folded(ids)) => {
                    {
                        let mut state = state.borrow_mut();
                        for id in &ids {
                            state.toggled.insert(*id, true);
                        }
                    }
                    fill(list, &state);
                    if let Some(first) = ids.first() {
                        focus_assembly(list, &state, *first);
                    }
                    let callback = state.borrow().on_select.clone();
                    if let (Some(callback), Some(first)) = (callback, ids.first()) {
                        callback(Selection::Assembly(*first));
                    }
                }
                Some(RowKind::Joined {
                    focus, selection, ..
                }) => match (focus, selection) {
                    (Some(target), _) => {
                        let callback = state.borrow().on_focus.clone();
                        if let Some(callback) = callback {
                            state.borrow_mut().clicked = Some(Selection::of_target(target));
                            callback(target);
                            state.borrow_mut().clicked = None;
                        }
                    }
                    (None, Some(selection)) => {
                        let callback = state.borrow().on_select.clone();
                        if let Some(callback) = callback {
                            callback(selection);
                        }
                    }
                    (None, None) => {}
                },
                Some(RowKind::Source(location)) => {
                    let callback = state.borrow().on_source.clone();
                    if let Some(callback) = callback {
                        callback(location);
                    }
                }
                Some(RowKind::Note) | None => {}
            }
        });
        // The arrow keys only move the row selection (a click or Enter
        // chooses); Right, Left, + and − open, fold and walk the tree.
        let keys = gtk::EventControllerKey::new();
        let state = self.state.clone();
        keys.connect_key_pressed(move |keys, key, _, modifiers| {
            use gtk::gdk::{Key, ModifierType};
            if modifiers.intersects(
                ModifierType::CONTROL_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK,
            ) {
                return gtk::glib::Propagation::Proceed;
            }
            let Some(list) = keys.widget().and_downcast::<gtk::ListBox>() else {
                return gtk::glib::Propagation::Proceed;
            };
            let rtl = list.direction() == gtk::TextDirection::Rtl;
            let (right, left) = if rtl {
                (Step::Shut, Step::Open)
            } else {
                (Step::Open, Step::Shut)
            };
            let step = match key {
                Key::Right | Key::KP_Right => right,
                Key::Left | Key::KP_Left => left,
                Key::plus | Key::KP_Add => Step::Open,
                Key::minus | Key::KP_Subtract => Step::Shut,
                _ => return gtk::glib::Propagation::Proceed,
            };
            let Some(row) = list.focus_child().and_downcast::<gtk::ListBoxRow>() else {
                return gtk::glib::Propagation::Proceed;
            };
            if step_row(&list, &state, &row, step) {
                gtk::glib::Propagation::Stop
            } else {
                gtk::glib::Propagation::Proceed
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

    /// Calls `callback` with what a partner link stands for while the
    /// pointer is over it, and with nothing when it leaves.
    pub fn connect_hover(&self, callback: impl Fn(Vec<Item>) + 'static) {
        self.state.borrow_mut().on_hover = Some(Rc::new(callback));
    }

    /// Shows a new tree and its joints, keeping what is folded.
    pub fn show_tree(&self, tree: PartsTree, joints: PartJoints) {
        {
            let mut state = self.state.borrow_mut();
            state.tree = tree;
            state.joints = joints;
            state.empty = None;
        }
        fill(&self.list, &self.state);
    }

    /// Shows why there are no parts; for a library, what it defines.
    pub fn show_empty(&self, empty: EmptyState) {
        {
            let mut state = self.state.borrow_mut();
            state.tree = PartsTree::default();
            state.joints = PartJoints::default();
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

    /// Highlights the selection's row and lists what the selected part is
    /// joined to under it. Choosing one of those rows keeps them listed
    /// under the same part, and then the rows are left as they are, so
    /// nothing folds and the list does not move.
    pub fn show_selection(&self, selection: Option<&Selection>) {
        let same_rows = {
            let mut state = self.state.borrow_mut();
            let listing = state
                .listing
                .next(selection, &state.joints, state.clicked.as_ref());
            let was_open = open_assemblies(&state);
            let was_listing = state.listing;
            let changed = state.selection.as_ref() != selection;
            state.selection = selection.cloned();
            state.listing = listing;
            if changed {
                reveal(&mut state);
            }
            let now_open = open_assemblies(&state);
            listing == was_listing && was_open == now_open
        };
        if same_rows {
            // The same selection again leaves the list's selection where
            // the keyboard put it.
            let changed = {
                let state = self.state.borrow();
                state.synced != state.selection
            };
            if changed {
                sync_selection(&self.list, &self.state);
            }
        } else {
            // The assembly holding it opens.
            fill(&self.list, &self.state);
        }
    }
}

/// The part whose joined-to rows the panel lists under its row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Listing {
    /// The part they are under.
    under: Option<Item>,
}

impl Listing {
    /// The listing once `selection` is chosen. A part lists its own rows;
    /// a piece of hardware or a joint's piece lists the rows of the part
    /// it is on. Choosing one of the listed rows (or what one stands for,
    /// from elsewhere, when it is a joint pair, which is not a part) keeps
    /// the listing where it is.
    fn next(
        &self,
        selection: Option<&Selection>,
        joints: &PartJoints,
        clicked: Option<&Selection>,
    ) -> Listing {
        let listed = |selection: &Selection| {
            self.under
                .and_then(|item| match item {
                    Item::Part(part) => Some(part),
                    Item::Hardware(_) | Item::Member(_) => None,
                })
                .is_some_and(|part| {
                    joints
                        .joined_to(part)
                        .iter()
                        .any(|row| row.stands_for.contains(selection))
                })
        };
        let Some(selection) = selection else {
            return Listing::default();
        };
        let keep = listed(selection) && (selection.item().is_none() || clicked == Some(selection));
        if keep {
            return *self;
        }
        Listing {
            under: selection
                .item()
                .and_then(|item| joints.home(item))
                .map(Item::Part),
        }
    }
}

/// Opens the assemblies the user folded that hold a part (or what a part
/// carries) chosen elsewhere, so its row shows. An assembly chosen itself
/// (its heading, a folded row) is left as the user left it.
fn reveal(state: &mut TreeState) {
    let selection = state.selection.clone().filter(|s| s.item().is_some());
    let under = state.listing.under.map(Selection::of_item);
    let mut stack: Vec<&AssemblyNode> = state.tree.assemblies.iter().collect();
    let mut opened = Vec::new();
    while let Some(node) = stack.pop() {
        let folded = state.toggled.get(&node.id) == Some(&false);
        if folded
            && [&selection, &under]
                .into_iter()
                .flatten()
                .any(|s| holds(node, s))
        {
            opened.push(node.id);
        }
        stack.extend(node.children.iter());
    }
    for id in opened {
        state.toggled.insert(id, true);
    }
}

/// What Right, Left, + or − asks of the row with the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Open it, or go into it when it is open.
    Open,
    /// Fold it, or go out to the row it is under.
    Shut,
}

/// Does `step` on `row`; `false` when it means nothing there.
fn step_row(
    list: &gtk::ListBox,
    state: &Rc<RefCell<TreeState>>,
    row: &gtk::ListBoxRow,
    step: Step,
) -> bool {
    let (index, kind) = {
        let state = state.borrow();
        let Some(index) = state.rows.iter().position(|(r, _)| r == row) else {
            return false;
        };
        (index, state.rows[index].1.clone())
    };
    // The nearest row above this one that `wanted` picks.
    let above = |wanted: &dyn Fn(&RowKind) -> bool| {
        let state = state.borrow();
        state.rows[..index]
            .iter()
            .rev()
            .find(|(_, kind)| wanted(kind))
            .map(|(row, _)| row.clone())
    };
    match (kind, step) {
        (RowKind::Heading(_), Step::Open) => {
            let next = state
                .borrow()
                .rows
                .get(index + 1)
                .map(|(row, _)| row.clone());
            next.is_some_and(|row| row.grab_focus())
        }
        (RowKind::Heading(id), Step::Shut) => {
            state.borrow_mut().toggled.insert(id, false);
            fill(list, state);
            focus_assembly(list, state, id);
            true
        }
        (RowKind::Folded(ids), Step::Open) => {
            {
                let mut state = state.borrow_mut();
                for id in &ids {
                    state.toggled.insert(*id, true);
                }
            }
            fill(list, state);
            if let Some(first) = ids.first() {
                focus_assembly(list, state, *first);
            }
            true
        }
        (RowKind::Entry { .. }, Step::Shut) => {
            above(&|kind| matches!(kind, RowKind::Heading(_))).is_some_and(|row| row.grab_focus())
        }
        (RowKind::Joined { .. }, Step::Shut) => {
            above(&|kind| matches!(kind, RowKind::Entry { .. })).is_some_and(|row| row.grab_focus())
        }
        (RowKind::Folded(_) | RowKind::Entry { .. } | RowKind::Joined { .. }, _)
        | (RowKind::Note | RowKind::Source(_), _) => false,
    }
}

/// Gives the keyboard to an assembly's heading or the folded row it is
/// in, and selects that row.
fn focus_assembly(list: &gtk::ListBox, state: &Rc<RefCell<TreeState>>, id: AssemblyId) {
    let row = state
        .borrow()
        .rows
        .iter()
        .find(|(_, kind)| match kind {
            RowKind::Heading(heading) => *heading == id,
            RowKind::Folded(ids) => ids.contains(&id),
            RowKind::Note | RowKind::Entry { .. } | RowKind::Joined { .. } | RowKind::Source(_) => {
                false
            }
        })
        .map(|(row, _)| row.clone());
    if let Some(row) = row {
        list.select_row(Some(&row));
        row.grab_focus();
    }
}

/// The assemblies whose rows show, in tree order.
fn open_assemblies(state: &TreeState) -> Vec<AssemblyId> {
    let mut open = Vec::new();
    let mut stack: Vec<&AssemblyNode> = state.tree.assemblies.iter().rev().collect();
    while let Some(node) = stack.pop() {
        if state.is_open(node) {
            open.push(node.id);
            stack.extend(node.children.iter().rev());
        }
    }
    open
}

/// Makes the rows again and puts in the list only those that read
/// differently from the ones it has: the rows the same at the start and
/// the end are kept, and only the run between is replaced.
fn fill(list: &gtk::ListBox, state: &Rc<RefCell<TreeState>>) {
    let mut state_ref = state.borrow_mut();
    let density = state_ref.density();
    if density.badge() {
        list.add_css_class("narrow");
    } else {
        list.remove_css_class("narrow");
    }
    let open = |id: AssemblyId| {
        find(&state_ref.tree.assemblies, id).is_some_and(|node| state_ref.is_open(node))
    };
    let blocks = state_ref.joints.blocks(&state_ref.tree, &open);
    let joined: &[JoinedTo] = match state_ref.listing.under {
        Some(Item::Part(part)) => state_ref.joints.joined_to(part),
        Some(Item::Hardware(_) | Item::Member(_)) | None => &[],
    };
    // The parts the selection is joined to, for their rows' blue edge.
    let partners: BTreeSet<Item> = joined
        .iter()
        .flat_map(|row| row.partners.iter().flat_map(|p| p.items.iter().copied()))
        .collect();
    let shown = Shown {
        joints: &state_ref.joints,
        marks: &state_ref.marks,
        under: state_ref.listing.under,
        joined,
        partners: &partners,
        density,
        on_select: state_ref.on_select.clone(),
        on_hover: state_ref.on_hover.clone(),
        clicked: state.clone(),
    };
    let mut rows = Vec::new();
    for block in &blocks {
        match block {
            Block::Heading { assembly, entries } => {
                rows.push(heading_row(&state_ref.joints, *assembly, density));
                for entry in entries {
                    entry_rows(entry, &shown, &mut rows);
                }
            }
            Block::Folded {
                assemblies,
                label,
                detail,
            } => {
                let ids: Vec<AssemblyId> = assemblies.iter().copied().collect();
                rows.push(folded_row(&ids, label, detail.as_deref(), &shown));
            }
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
        let row = tree_row()
            .child(&label)
            .activatable(false)
            .selectable(false)
            .build();
        rows.push((row, RowKind::Note));
    }
    drop(shown);
    for (row, kind) in &rows {
        row.update_property(&[gtk::accessible::Property::Level(level(kind))]);
    }
    let prints: Vec<String> = rows.iter().map(|(row, kind)| print(row, kind)).collect();
    let mut kept = std::mem::take(&mut state_ref.rows);
    let old_prints = std::mem::take(&mut state_ref.prints);
    let start = old_prints
        .iter()
        .zip(&prints)
        .take_while(|(old, new)| old == new)
        .count();
    let end = old_prints[start..]
        .iter()
        .rev()
        .zip(prints[start..].iter().rev())
        .take_while(|(old, new)| old == new)
        .count();
    let suffix = kept.split_off(kept.len() - end);
    let replaced = kept.len() > start || rows.len() > start + end;
    for (row, _) in kept.drain(start..) {
        list.remove(&row);
    }
    let middle = rows.len() - start - end;
    for (offset, (row, kind)) in rows.into_iter().skip(start).take(middle).enumerate() {
        list.insert(&row, i32::try_from(start + offset).unwrap_or(-1));
        kept.push((row, kind));
    }
    kept.extend(suffix);
    state_ref.rows = kept;
    state_ref.prints = prints;
    let resync = replaced || state_ref.synced != state_ref.selection;
    let on_hover = state_ref.on_hover.clone();
    drop(state_ref);
    if resync {
        sync_selection(list, state);
    }
    // A link under the pointer may have gone with the old rows without
    // saying it was left.
    if replaced && let Some(on_hover) = on_hover {
        on_hover(Vec::new());
    }
}

/// A row of the tree: a tree item to screen readers.
fn tree_row() -> gtk::builders::ListBoxRowBuilder {
    gtk::ListBoxRow::builder().accessible_role(gtk::AccessibleRole::TreeItem)
}

/// A row's level in the tree, for screen readers.
fn level(kind: &RowKind) -> i32 {
    match kind {
        RowKind::Note | RowKind::Heading(_) | RowKind::Folded(_) => 1,
        RowKind::Entry { .. } | RowKind::Source(_) => 2,
        RowKind::Joined { .. } => 3,
    }
}

/// Names a row for screen readers and describes it (its dim text and its
/// mark in words, since the dot is only a colour). The row's widget name
/// keeps both, so a row whose words changed reads differently to `print`.
fn name_row(row: &gtk::ListBoxRow, label: &str, description: &str) {
    row.update_property(&[
        gtk::accessible::Property::Label(label),
        gtk::accessible::Property::Description(description),
    ]);
    row.set_widget_name(&format!("{label}\n{description}"));
}

/// What a screen reader hears after a row's name: its dim text and its
/// mark in words.
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

/// Everything a row shows, as text: two rows that print the same look and
/// act the same, so the list keeps the one it has.
fn print(row: &gtk::ListBoxRow, kind: &RowKind) -> String {
    let mut out = format!("{kind:?} {} {}", row.is_activatable(), row.is_selectable());
    print_widget(row.upcast_ref(), &mut out);
    out
}

fn print_widget(widget: &gtk::Widget, out: &mut String) {
    use std::fmt::Write as _;
    let _ = write!(
        out,
        "<{} {} {:?} {:?} {}",
        widget.type_().name(),
        widget.widget_name(),
        widget.css_classes(),
        widget.tooltip_text(),
        widget.is_visible(),
    );
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        let _ = write!(out, " {:?} {}", label.label(), label.width_chars());
    }
    if let Some(icon) = widget.downcast_ref::<PathIcon>() {
        let _ = write!(out, " {:?}", icon.shape());
    }
    if let Some(container) = widget.downcast_ref::<gtk::Box>() {
        let _ = write!(out, " {:?} {}", container.halign(), container.hexpands());
    }
    let mut child = widget.first_child();
    while let Some(next) = child {
        print_widget(&next, out);
        child = next.next_sibling();
    }
    out.push('>');
}

fn sync_selection(list: &gtk::ListBox, state_cell: &Rc<RefCell<TreeState>>) {
    let mut state = state_cell.borrow_mut();
    state.synced = state.selection.clone();
    let selected = state.selection.as_ref().and_then(|selection| {
        let find = |wanted: &dyn Fn(&RowKind) -> bool| {
            state
                .rows
                .iter()
                .find(|(_, kind)| wanted(kind))
                .map(|(row, _)| row.clone())
        };
        // A part's own row; else the joined-to row that stands for it (a
        // piece of hardware, a joint pair or its pins); else an
        // assembly's heading or folded row, or a library's item.
        find(&|kind| match kind {
            RowKind::Entry { items, .. } => {
                selection.item().is_some_and(|item| items.contains(&item))
            }
            RowKind::Note
            | RowKind::Heading(_)
            | RowKind::Folded(_)
            | RowKind::Joined { .. }
            | RowKind::Source(_) => false,
        })
        .or_else(|| {
            find(&|kind| match kind {
                RowKind::Joined { stands_for, .. } => stands_for.contains(selection),
                RowKind::Note
                | RowKind::Heading(_)
                | RowKind::Folded(_)
                | RowKind::Entry { .. }
                | RowKind::Source(_) => false,
            })
        })
        .or_else(|| {
            find(&|kind| match kind {
                RowKind::Heading(id) => *selection == Selection::Assembly(*id),
                RowKind::Folded(ids) => ids.iter().any(|id| *selection == Selection::Assembly(*id)),
                RowKind::Source(location) => *selection == Selection::Location(location.clone()),
                RowKind::Note | RowKind::Entry { .. } | RowKind::Joined { .. } => false,
            })
        })
    });
    drop(state);
    match selected {
        Some(row) => list.select_row(Some(&row)),
        None => list.unselect_all(),
    }
}

/// What the rows need besides the tree itself.
struct Shown<'a> {
    joints: &'a PartJoints,
    marks: &'a Marks,
    /// The part whose joined-to rows show.
    under: Option<Item>,
    joined: &'a [JoinedTo],
    partners: &'a BTreeSet<Item>,
    density: Density,
    on_select: Option<SelectCallback>,
    on_hover: Option<HoverCallback>,
    /// The panel's state, to say which joined-to row a partner link was
    /// clicked in.
    clicked: Rc<RefCell<TreeState>>,
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
            tree_row()
                .child(&label)
                .activatable(false)
                .selectable(false)
                .build(),
            RowKind::Note,
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
        rows.push(plain_heading(group.heading()));
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
        .css_classes(["stave-parts-entry", "stave-library-entry"])
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
                .css_classes(["detail"])
                .build(),
        );
    }
    let row = tree_row().child(&content).build();
    name_row(
        &row,
        &item.name,
        item.summary.as_deref().unwrap_or_default(),
    );
    let kind = match &item.location {
        Some(location) => {
            row.set_tooltip_text(Some(item.summary.as_deref().unwrap_or("Open its text")));
            RowKind::Source(location.clone())
        }
        None => {
            row.set_activatable(false);
            row.set_selectable(false);
            RowKind::Note
        }
    };
    (row, kind)
}

/// A heading with only its words (a library's groups).
fn plain_heading(text: &str) -> (gtk::ListBoxRow, RowKind) {
    let content = gtk::Box::builder()
        .css_classes(["stave-parts-heading"])
        .build();
    content.append(&gtk::Label::builder().label(text).xalign(0.0).build());
    let row = tree_row()
        .child(&content)
        .activatable(false)
        .selectable(false)
        .build();
    (row, RowKind::Note)
}

/// An assembly's (or the carcass's) heading: its name, then its motion
/// icon and grey note.
fn heading_row(
    joints: &PartJoints,
    assembly: Option<AssemblyId>,
    density: Density,
) -> (gtk::ListBoxRow, RowKind) {
    let content = gtk::Box::builder()
        .css_classes(["stave-parts-heading"])
        .build();
    let Some(heading) = joints.heading(assembly) else {
        return plain_heading("");
    };
    // An assembly's heading folds it: a small open chevron says so.
    if assembly.is_some() {
        let chevron = PathIcon::new(icons::EXPANDED, 10);
        chevron.add_css_class("stave-glyph-plain");
        content.append(&chevron);
    }
    content.append(
        &gtk::Label::builder()
            .label(&heading.title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    if let Some(motion) = heading.motion {
        let icon = PathIcon::new(
            match motion {
                Motion::Turn => TURNS,
                Motion::Travel => SLIDES,
            },
            12,
        );
        icon.add_css_class("stave-glyph-motion");
        let words = heading.motion_words.as_deref().unwrap_or(&heading.note);
        icon.set_tooltip_text(Some(words));
        icon.update_property(&[gtk::accessible::Property::Label(words)]);
        content.append(&icon);
    }
    if density.heading_notes() {
        content.append(
            &gtk::Label::builder()
                .label(&heading.note)
                .css_classes(["note"])
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
    }
    let row = tree_row().child(&content).build();
    name_row(&row, &heading.title, &heading.note);
    match assembly {
        Some(id) => {
            row.set_tooltip_text(Some("Fold it"));
            row.update_state(&[gtk::accessible::State::Expanded(Some(true))]);
            (row, RowKind::Heading(id))
        }
        None => {
            row.set_activatable(false);
            row.set_selectable(false);
            (row, RowKind::Note)
        }
    }
}

/// A row's chips, as chips or (narrow) one badge.
fn chips_box(chips: &[Chip], density: Density) -> Option<gtk::Box> {
    if chips.is_empty() {
        return None;
    }
    let content = gtk::Box::builder()
        .css_classes(["stave-chips"])
        .valign(gtk::Align::Center)
        .build();
    if density.badge() {
        let (kinds, tooltip) = part_joints::badge(chips)?;
        let badge = gtk::Box::builder()
            .css_classes(["stave-chip"])
            .tooltip_text(&tooltip)
            .build();
        let icon = PathIcon::new(JOINTS, 12);
        icon.add_css_class("stave-glyph-plain");
        badge.append(&icon);
        badge.append(&gtk::Label::new(Some(&kinds.to_string())));
        badge.update_property(&[gtk::accessible::Property::Label(&tooltip)]);
        content.append(&badge);
        return Some(content);
    }
    for chip in chips {
        let tooltip = chip.tooltip();
        let widget = gtk::Box::builder()
            .css_classes(["stave-chip"])
            .tooltip_text(&tooltip)
            .build();
        widget.append(&glyph_icon(chip.glyph, 14, &tooltip));
        if density.chip_counts() && chip.shows_count() {
            widget.append(&gtk::Label::new(Some(&chip.count.to_string())));
        }
        content.append(&widget);
    }
    Some(content)
}

/// The rows of an entry: one, or one per member when the part whose
/// joined-to rows show is one of its members (so each can be chosen);
/// the joined-to rows follow that part's row.
fn entry_rows(entry: &TreeEntry, shown: &Shown<'_>, rows: &mut Vec<(gtk::ListBoxRow, RowKind)>) {
    let under = shown.under;
    let split =
        entry.items.len() > 1 && under.is_some_and(|item| entry.items.iter().any(|i| *i == item));
    if split {
        for (item, label) in entry.items.iter().zip(&entry.member_labels) {
            rows.push(entry_row(entry, label, vec![*item], shown));
            if under == Some(*item) {
                joined_rows(shown, rows);
            }
        }
    } else {
        rows.push(entry_row(
            entry,
            &entry.label,
            entry.items.iter().copied().collect(),
            shown,
        ));
        if under.is_some_and(|item| entry.items.iter().any(|i| *i == item)) {
            joined_rows(shown, rows);
        }
    }
}

fn entry_row(
    entry: &TreeEntry,
    label: &str,
    items: Vec<Item>,
    shown: &Shown<'_>,
) -> (gtk::ListBoxRow, RowKind) {
    let content = gtk::Box::builder()
        .css_classes(["stave-parts-entry"])
        .build();
    let swatch = icons::swatch(
        rgba(entry.swatch),
        entry.swatch.shape == SwatchShape::Round,
        9,
    );
    // Its colour, for `print`: the drawing shows it nowhere else.
    swatch.set_widget_name(&format!("{:?}", entry.swatch));
    content.append(&swatch);
    content.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let chips = shown.joints.chips(&items);
    if let Some(chips) = chips_box(&chips, shown.density) {
        content.append(&chips);
    }
    let mark = if items.len() == 1 {
        items.first().and_then(|item| shown.marks.item(*item))
    } else {
        shown.marks.entry(entry)
    };
    content.append(&super::style::mark_dot(mark));
    let row = tree_row().child(&content).build();
    if let Some(detail) = &entry.detail {
        row.set_tooltip_text(Some(detail));
    }
    if items.iter().any(|item| shown.partners.contains(item)) {
        row.add_css_class("partner");
    }
    let words: Vec<String> = chips.iter().map(Chip::tooltip).collect();
    let accessible = if words.is_empty() {
        label.to_owned()
    } else {
        format!("{label}: {}", words.join(", "))
    };
    name_row(&row, &accessible, &described(entry.detail.as_deref(), mark));
    let first = items.first().copied().unwrap_or(*entry.items.first());
    (
        row,
        RowKind::Entry {
            selection: Selection::of_item(first),
            items,
        },
    )
}

/// The joined-to rows of the part they are listed under.
fn joined_rows(shown: &Shown<'_>, rows: &mut Vec<(gtk::ListBoxRow, RowKind)>) {
    for joined in shown.joined {
        rows.push(joined_row(joined, shown));
    }
}

fn joined_row(joined: &JoinedTo, shown: &Shown<'_>) -> (gtk::ListBoxRow, RowKind) {
    let content = gtk::Box::builder()
        .css_classes(["stave-parts-joined"])
        .build();
    content.append(&glyph_icon(joined.glyph, 14, &joined.words));
    let words = gtk::Label::builder()
        .label(&joined.words)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["words"])
        .build();
    if shown.density.joined_words() {
        content.append(&words);
    } else {
        content.set_tooltip_text(Some(&joined.words));
    }
    let partners = gtk::Box::builder()
        .spacing(3)
        .halign(if shown.density.joined_words() {
            gtk::Align::End
        } else {
            gtk::Align::Start
        })
        .hexpand(!shown.density.joined_words())
        .build();
    for partner in &joined.partners {
        partners.append(&partner_link(partner, joined, shown));
    }
    content.append(&partners);
    let row = tree_row().child(&content).build();
    let names: Vec<&str> = joined.partners.iter().map(|p| p.label.as_str()).collect();
    let accessible = format!("{}: {}", joined.words, names.join(", "));
    name_row(&row, &accessible, "");
    if joined.focus.is_some() {
        row.set_tooltip_text(Some(&format!("{accessible}. Activate to focus on it")));
    }
    (
        row,
        RowKind::Joined {
            focus: joined.focus,
            stands_for: joined.stands_for.clone(),
            selection: joined
                .partners
                .iter()
                .find_map(|partner| partner.selection.clone()),
        },
    )
}

/// A partner's name as a link: clicking selects it, hovering lights it in
/// the 3D view. A space, which can't be selected, is plain text.
fn partner_link(partner: &Partner, joined: &JoinedTo, shown: &Shown<'_>) -> gtk::Widget {
    // The name keeps up to a few characters before the joint's words give
    // way, then is cut short itself.
    let keeps = partner
        .label
        .chars()
        .count()
        .min(shown.density.partner_chars());
    let label = gtk::Label::builder()
        .label(&partner.label)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .width_chars(i32::try_from(keeps).unwrap_or(0))
        .build();
    let Some(selection) = partner.selection.clone() else {
        label.add_css_class("stave-partner-text");
        return label.upcast();
    };
    let button = gtk::Button::builder()
        .child(&label)
        .css_classes(["stave-partner", "flat"])
        .valign(gtk::Align::Center)
        .tooltip_text(format!("Select {}", partner.label))
        .build();
    let on_select = shown.on_select.clone();
    let state = shown.clicked.clone();
    let row_selection = joined.stands_for.first().cloned();
    button.connect_clicked(move |_| {
        if let Some(callback) = &on_select {
            // A piece of hardware chosen here keeps the rows where they
            // are, as if its row were activated.
            state.borrow_mut().clicked = row_selection
                .clone()
                .filter(|row| *row == selection)
                .or_else(|| Some(selection.clone()));
            callback(selection.clone());
            state.borrow_mut().clicked = None;
        }
    });
    if let Some(on_hover) = shown.on_hover.clone() {
        let motion = gtk::EventControllerMotion::new();
        let items = partner.items.clone();
        let entered = on_hover.clone();
        motion.connect_enter(move |_, _, _| entered(items.clone()));
        motion.connect_leave(move |_| on_hover(Vec::new()));
        button.add_controller(motion);
    }
    button.upcast()
}

/// A row for folded assemblies: a chevron, the name, what it is ("mirror
/// of door_l") and its chips.
fn folded_row(
    ids: &[AssemblyId],
    label: &str,
    detail: Option<&str>,
    shown: &Shown<'_>,
) -> (gtk::ListBoxRow, RowKind) {
    let content = gtk::Box::builder()
        .css_classes(["stave-parts-folded"])
        .build();
    let chevron = PathIcon::new(icons::COLLAPSED, 12);
    chevron.add_css_class("stave-glyph-plain");
    content.append(&chevron);
    content.append(
        &gtk::Label::builder()
            .label(label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let chips = shown.joints.folded_chips(ids);
    if let Some(chips) = chips_box(&chips, shown.density) {
        content.append(&chips);
    }
    if let Some(detail) = detail.filter(|_| shown.density.folded_detail()) {
        content.append(
            &gtk::Label::builder()
                .label(detail)
                .css_classes(["detail"])
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
    }
    let mark = ids.iter().find_map(|id| shown.marks.assembly(*id));
    content.append(&super::style::mark_dot(mark));
    let row = tree_row().child(&content).tooltip_text("Open it").build();
    name_row(&row, label, &described(detail, mark));
    row.update_state(&[gtk::accessible::State::Expanded(Some(false))]);
    (row, RowKind::Folded(ids.to_vec()))
}

/// The node for an assembly, anywhere in `nodes`.
fn find(nodes: &[AssemblyNode], id: AssemblyId) -> Option<&AssemblyNode> {
    let mut stack: Vec<&AssemblyNode> = nodes.iter().collect();
    while let Some(node) = stack.pop() {
        if node.id == id {
            return Some(node);
        }
        stack.extend(node.children.iter());
    }
    None
}

/// Whether an assembly shows as a heading with its rows: as the user left
/// it; else open when it holds the selection or the part whose joined-to
/// rows show (`under`); else unless it folds by default.
fn is_open(
    node: &AssemblyNode,
    joints: &PartJoints,
    toggled: &BTreeMap<AssemblyId, bool>,
    selection: Option<&Selection>,
    under: Option<Item>,
) -> bool {
    toggled.get(&node.id).copied().unwrap_or_else(|| {
        !joints.folded_by_default(node.id)
            || selection.is_some_and(|s| holds(node, s))
            || under.is_some_and(|item| holds(node, &Selection::of_item(item)))
    })
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
    use stave_model::solved::SolvedDesign;
    use std::path::Path;

    fn cabinet() -> SolvedDesign {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
        stave_app::open(&folder)
            .expect("the cabinet opens")
            .design
            .expect("the cabinet solves")
    }

    fn part(design: &SolvedDesign, name: &str) -> Item {
        Item::Part(design.part_named(name).expect("the part exists").id)
    }

    #[test]
    fn selecting_a_part_lists_its_rows_under_it() {
        let design = cabinet();
        let joints = PartJoints::new(&design);
        let stile = part(&design, "door_l.stile[hinge]");
        let listing = Listing::default().next(Some(&Selection::of_item(stile)), &joints, None);
        assert_eq!(listing.under, Some(stile));
        assert_eq!(
            Listing::default().next(None, &joints, None),
            Listing::default()
        );
    }

    #[test]
    fn hardware_lists_the_part_it_is_on() {
        let design = cabinet();
        let joints = PartJoints::new(&design);
        let hinge = design
            .hardware
            .iter()
            .find(|h| h.name == "door_l.hinges[lower]")
            .expect("the hinge exists");
        let listing = Listing::default().next(Some(&Selection::Hardware(hinge.id)), &joints, None);
        assert_eq!(listing.under, Some(part(&design, "door_l.stile[hinge]")));
    }

    #[test]
    fn choosing_a_listed_row_keeps_the_listing() {
        let design = cabinet();
        let joints = PartJoints::new(&design);
        let side = part(&design, "side_l");
        let listing = Listing::default().next(Some(&Selection::of_item(side)), &joints, None);
        // A joint pair is not a part: it stays under side_l, from wherever.
        let Some(Item::Part(side_id)) = listing.under else {
            panic!("side_l is listed");
        };
        let pair = joints
            .joined_to(side_id)
            .first()
            .and_then(|row| row.stands_for.first().cloned())
            .expect("side_l has a joint");
        assert_eq!(listing.next(Some(&pair), &joints, None), listing);
        // door_l's hinges, clicked under side_l, stay under side_l too;
        // chosen elsewhere they list the stile they are on.
        let hinge = joints
            .joined_to(side_id)
            .iter()
            .find(|row| row.glyph == Glyph::Hinge)
            .and_then(|row| row.stands_for.first().cloned())
            .expect("side_l lists the hinges");
        assert_eq!(listing.next(Some(&hinge), &joints, Some(&hinge)), listing);
        assert_eq!(
            listing.next(Some(&hinge), &joints, None).under,
            Some(part(&design, "door_l.stile[hinge]"))
        );
    }

    /// The panel alone at a few widths, dark, with `door_l.stile[hinge]`
    /// selected, saved as `parts-<width>.png` in `STAVE_PARTS_SHOTS` (the
    /// window's layout file keeps no widths, so a whole-window shot can't
    /// make the left edge narrow). Run it on a display:
    /// `STAVE_PARTS_SHOTS=dir cargo test -p stave-gtk parts_panel_pictures
    /// -- --ignored`.
    #[test]
    #[ignore = "writes pictures; needs a display"]
    fn parts_panel_pictures() {
        let Some(folder) = std::env::var_os("STAVE_PARTS_SHOTS") else {
            return;
        };
        crate::ui::gtk_test::on_gtk_thread(move || {
            let design = cabinet();
            let display = gtk::gdk::Display::default().expect("a display");
            crate::ui::style::install(&display);
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
            let stile = part(&design, "door_l.stile[hinge]");
            for width in [300, 200, 150] {
                let sidebar = TreeSidebar::new();
                sidebar.show_tree(
                    stave_app::inspect::parts_tree(&design),
                    PartJoints::new(&design),
                );
                sidebar.show_selection(Some(&Selection::of_item(stile)));
                let window = gtk::Window::builder()
                    .default_width(width)
                    .default_height(560)
                    .decorated(false)
                    .css_classes(["stave-dark", "background"])
                    .child(&sidebar.root)
                    .build();
                // The paintable follows the window's drawing from now on.
                let paintable = gtk::WidgetPaintable::new(Some(&window));
                window.present();
                let context = gtk::glib::MainContext::default();
                // A few rounds, each drawing the window again, so the rows
                // the width watcher asks for are drawn.
                for _ in 0..12 {
                    let done = Rc::new(Cell::new(false));
                    let flag = done.clone();
                    gtk::glib::timeout_add_local_once(
                        std::time::Duration::from_millis(250),
                        move || flag.set(true),
                    );
                    window.queue_draw();
                    window.add_tick_callback(|_, _| gtk::glib::ControlFlow::Break);
                    while !done.get() {
                        context.iteration(true);
                    }
                }
                let snapshot = gtk::Snapshot::new();
                paintable.snapshot(
                    &snapshot,
                    f64::from(window.width()),
                    f64::from(window.height()),
                );
                let node = snapshot.to_node().expect("the panel drew");
                let renderer = gtk::gsk::CairoRenderer::new();
                renderer.realize_for_display(&display).expect("a renderer");
                let texture = renderer.render_texture(&node, None);
                renderer.unrealize();
                let path = Path::new(&folder).join(format!("parts-{width}.png"));
                texture.save_to_png(&path).expect("the picture saves");
                window.destroy();
            }
        });
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
        sidebar.show_tree(
            stave_app::inspect::parts_tree(design),
            stave_app::part_joints::PartJoints::new(design),
        );
        let part = design.parts.first().expect("a part");
        let selection = Selection::of_item(Item::Part(part.id));
        sidebar.show_selection(Some(&selection));
        assert!(
            sidebar.list.selected_row().is_some(),
            "the part's row is selected"
        );
        assert!(
            chosen.borrow().is_empty(),
            "the tree's own change is no choice"
        );
        // Showing it again leaves the rows as they are and syncs only.
        sidebar.show_selection(Some(&selection));
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
        // The arrow keys only move the list's selection: that is no
        // choice. Enter or a click (activating the row) is.
        sidebar.list.select_row(Some(&other));
        assert!(
            chosen.borrow().is_empty(),
            "moving through the rows chooses nothing"
        );
        sidebar.list.emit_by_name::<()>("row-activated", &[&other]);
        assert_eq!(
            chosen.borrow().len(),
            1,
            "a row chosen by hand is passed on"
        );
    }

    fn cabinet_sidebar() -> (TreeSidebar, SolvedDesign) {
        let design = cabinet();
        let sidebar = TreeSidebar::new();
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            PartJoints::new(&design),
        );
        (sidebar, design)
    }

    fn list_rows(sidebar: &TreeSidebar) -> Vec<gtk::ListBoxRow> {
        let mut rows = Vec::new();
        let mut index = 0;
        while let Some(row) = sidebar.list.row_at_index(index) {
            rows.push(row);
            index += 1;
        }
        rows
    }

    #[test]
    fn a_new_result_updates_rows_in_place() {
        crate::ui::gtk_test::on_gtk_thread(rows_update_in_place);
    }

    /// The same tree, joints or marks again keep every row; a selection
    /// that lists a part's joined-to rows replaces only the run around
    /// it. The list is a tree to screen readers.
    fn rows_update_in_place() {
        let (sidebar, design) = cabinet_sidebar();
        assert_eq!(sidebar.list.accessible_role(), gtk::AccessibleRole::Tree);
        let before = list_rows(&sidebar);
        assert!(before.len() > 4);
        assert!(
            before
                .iter()
                .all(|row| row.accessible_role() == gtk::AccessibleRole::TreeItem)
        );
        let state_rows: Vec<gtk::ListBoxRow> = sidebar
            .state
            .borrow()
            .rows
            .iter()
            .map(|(row, _)| row.clone())
            .collect();
        assert_eq!(state_rows, before);
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            PartJoints::new(&design),
        );
        sidebar.show_marks(Rc::new(Marks::default()));
        assert_eq!(
            list_rows(&sidebar),
            before,
            "an unchanged tree made rows again"
        );
        let side = part(&design, "side_l");
        sidebar.show_selection(Some(&Selection::of_item(side)));
        let after = list_rows(&sidebar);
        assert!(
            after.len() > before.len(),
            "side_l lists what it is joined to"
        );
        let first = before.first().expect("rows");
        let last = before.last().expect("rows");
        assert_eq!(after.first(), Some(first), "the first row was made again");
        assert_eq!(after.last(), Some(last), "the last row was made again");
        let state_rows: Vec<gtk::ListBoxRow> = sidebar
            .state
            .borrow()
            .rows
            .iter()
            .map(|(row, _)| row.clone())
            .collect();
        assert_eq!(state_rows, after);
    }

    #[test]
    fn left_folds_a_heading_and_right_opens_it() {
        crate::ui::gtk_test::on_gtk_thread(keys_fold_and_open);
    }

    /// Left (−) folds an assembly's heading, Right (+) opens the folded
    /// row again, Left on a part row goes to its heading; none of it is a
    /// choice.
    fn keys_fold_and_open() {
        let (sidebar, _design) = cabinet_sidebar();
        let chosen = Rc::new(Cell::new(0));
        sidebar.connect_select({
            let chosen = Rc::clone(&chosen);
            move |_| chosen.set(chosen.get() + 1)
        });
        let window = gtk::Window::builder().child(&sidebar.root).build();
        window.present();
        // The widget with the keyboard (`has_focus` needs the window to
        // be the active one, which a test's window may not be).
        let focus = || gtk::prelude::GtkWindowExt::focus(&window);
        let find_row = |wanted: &dyn Fn(&RowKind) -> bool| {
            sidebar
                .state
                .borrow()
                .rows
                .iter()
                .position(|(_, kind)| wanted(kind))
                .map(|index| (index, sidebar.state.borrow().rows[index].0.clone()))
        };
        let (index, heading) =
            find_row(&|kind| matches!(kind, RowKind::Heading(_))).expect("an assembly's heading");
        let RowKind::Heading(id) = sidebar.state.borrow().rows[index].1.clone() else {
            unreachable!();
        };
        // Left on the part row after it goes up to the heading.
        let entry = sidebar.state.borrow().rows[index + 1].0.clone();
        assert!(matches!(
            sidebar.state.borrow().rows[index + 1].1,
            RowKind::Entry { .. }
        ));
        assert!(step_row(&sidebar.list, &sidebar.state, &entry, Step::Shut));
        assert_eq!(focus().as_ref(), Some(heading.upcast_ref::<gtk::Widget>()));
        // Left on the heading folds it.
        assert!(step_row(
            &sidebar.list,
            &sidebar.state,
            &heading,
            Step::Shut
        ));
        let (_, folded) =
            find_row(&|kind| matches!(kind, RowKind::Folded(ids) if ids.contains(&id)))
                .expect("the assembly folded");
        assert_eq!(
            focus().as_ref(),
            Some(folded.upcast_ref::<gtk::Widget>()),
            "the folded row has the keyboard"
        );
        assert!(find_row(&|kind| matches!(kind, RowKind::Heading(h) if *h == id)).is_none());
        // Right opens it again.
        assert!(step_row(&sidebar.list, &sidebar.state, &folded, Step::Open));
        let (_, heading) = find_row(&|kind| matches!(kind, RowKind::Heading(h) if *h == id))
            .expect("the assembly opened");
        assert_eq!(focus().as_ref(), Some(heading.upcast_ref::<gtk::Widget>()));
        assert_eq!(chosen.get(), 0, "opening and folding chose nothing");
        window.destroy();
    }
}
