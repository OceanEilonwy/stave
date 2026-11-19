//! The Parts panel ("joint chips"): a flat list grouped by assembly. Each
//! group has a slim heading ("Carcass", `door_l`, drawn in capitals by
//! the `.stave-caps` style so the words stay as written) with a grey note
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
//!
//! The panel is a relm4 component, [`TreeComponent`] (the pattern is in
//! docs/relm4-migration.md): its model is the only state, only its update
//! changes it, and the rows' and the list's handlers only send
//! [`TreeInput`] messages. The rows themselves are plain widgets the update
//! builds, not components of their own. Selecting a row is no choice
//! (only activating one is), so the update selects rows itself with
//! nothing to block. [`TreeSidebar`] keeps the older callback API in front
//! of it until the window moves to relm4 too.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use adw::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};
use stave_app::empty::{EmptyState, LibraryItem};
use stave_app::focus::Target;
use stave_app::i18n::{fill_in, tr};
use stave_app::inspect::{AssemblyNode, PartsTree, Swatch, SwatchShape, TreeEntry};
use stave_app::part_joints::{
    self, Block, Chip, Density, Glyph, JoinedTo, Motion, PartJoints, Partner,
};
use stave_app::problems::{Mark, Marks};
use stave_app::session::{Selection, SourceLocation};
use stave_model::solved::AssemblyId;
use stave_scene::Item;

use super::icons::{self, Paint, PathIcon, Shape};
use super::relm4_shim::Bridge;

/// The panel's own look: sizes, the chips' colours (light, and dark under
/// the window's `stave-dark`) and the partner links.
const CSS: &str = r#"
.stave-parts { background: transparent; font-size: 0.852rem; }
.stave-parts > row { padding: 0; min-height: 0; margin: 0; border-radius: 0; }
.stave-parts > row:hover { background: alpha(currentColor, 0.06); }
.stave-parts > row:selected { background: @accent_bg_color; color: @accent_fg_color; }
.stave-parts > row.partner { box-shadow: inset 2px 0 0 #1c71d8; background: alpha(#3584e4, 0.08); }
.stave-dark .stave-parts > row.partner { box-shadow: inset 2px 0 0 #99c1f1; }
.stave-parts-heading { min-height: 22px; padding: 0 8px; margin-top: 4px; border-spacing: 4px; font-size: 0.716rem; font-weight: 700; letter-spacing: 0.04em; color: alpha(currentColor, 0.62); }
.stave-parts-heading .note { font-weight: 500; letter-spacing: 0; }
.stave-parts-entry { min-height: 24px; padding: 0 8px; border-spacing: 7px; }
.stave-parts-entry .detail { font-size: 0.784rem; color: alpha(currentColor, 0.62); }
.stave-parts-folded { min-height: 24px; padding: 0 8px 0 4px; border-spacing: 5px; }
.stave-parts-joined { min-height: 22px; padding: 0 8px 0 26px; border-spacing: 6px; font-size: 0.784rem; background: alpha(#3584e4, 0.12); }
.stave-parts-joined .words { color: alpha(currentColor, 0.66); }
.stave-parts.narrow .stave-parts-joined { padding-left: 14px; }
.stave-chips { border-spacing: 3px; }
.stave-chip { min-height: 18px; padding: 0 4px; border-radius: 4px; border-spacing: 2px; background: alpha(currentColor, 0.06); }
.stave-chip label { font-size: 0.716rem; color: alpha(currentColor, 0.72); }
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
button.stave-partner { min-height: 18px; min-width: 0; padding: 0 5px; border-radius: 4px; font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; font-weight: 400; color: #1c71d8; background: alpha(#3584e4, 0.10); }
button.stave-partner:hover { background: alpha(#3584e4, 0.24); }
.stave-dark button.stave-partner { color: #99c1f1; background: alpha(#3584e4, 0.16); }
.stave-dark button.stave-partner:hover { background: alpha(#3584e4, 0.30); }
.stave-partner-text { font-family: var(--monospace-font-family, monospace); font-size: 0.784rem; color: alpha(currentColor, 0.75); padding: 0 2px; }
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
        directional: false,
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

/// The Parts panel's state: the tree it shows, its rows and what is
/// folded, selected and listed. Only [`TreeComponent::update_with_view`]
/// changes it.
#[derive(Default)]
pub struct TreeComponent {
    tree: PartsTree,
    joints: PartJoints,
    /// The rows on show, in order, with what each stands for.
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
    /// What people last chose from a joined-to row (activating it, or
    /// clicking one of its partners), until the window next shows a
    /// selection: the selection that choice makes keeps the rows listed
    /// where it was chosen.
    clicked: Option<Selection>,
    /// How much the rows show at the panel's width.
    density: Option<Density>,
    /// Why there are no parts, when there are none.
    empty: Option<EmptyState>,
    /// What an empty tree says instead of "Nothing to show yet" (a new
    /// project's envelope: where to click).
    placeholder: Option<String>,
}

/// What the Parts panel is told: what to show by the window, and what
/// people did by its own widgets.
#[derive(Debug)]
pub enum TreeInput {
    /// Show a new tree and its joints, keeping what is folded.
    ShowTree {
        /// The parts, by assembly.
        tree: PartsTree,
        /// What each part is joined to.
        joints: PartJoints,
    },
    /// Show why there are no parts; for a library, what it defines.
    ShowEmpty(EmptyState),
    /// What the tree says while it has no rows: `None` for "Nothing to
    /// show yet".
    SetPlaceholder(Option<String>),
    /// The latest check's marks, for the dots.
    ShowMarks(Rc<Marks>),
    /// Highlight the selection's row and list what the selected part is
    /// joined to under it. Never passed back as a choice.
    ShowSelection(Option<Selection>),
    /// People activated a row (clicked it, or pressed Enter on it).
    Activated(gtk::ListBoxRow),
    /// People pressed Right, Left, + or − on the row with the keyboard.
    Step {
        /// The row with the keyboard.
        row: gtk::ListBoxRow,
        /// What the key asks of it.
        step: Step,
    },
    /// People clicked a partner link that selects this.
    PartnerClicked(Selection),
    /// The pointer went onto a partner link standing for these (empty: it
    /// left).
    Hovered(Vec<Item>),
    /// The panel is now this many pixels wide.
    Resized(i32),
}

/// What the Parts panel tells its owner.
#[derive(Clone, Debug, PartialEq)]
pub enum TreeOutput {
    /// People chose a row (or a partner link) standing for this.
    Select(Selection),
    /// People chose something a library defines: where it is written.
    Source(SourceLocation),
    /// People activated a joined-to row: focus mode on this joint or piece
    /// of hardware.
    Focus(Target),
    /// What the pointer is over in the panel (empty when it leaves, or
    /// when the rows are rebuilt under it).
    Hover(Vec<Item>),
}

impl TreeComponent {
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

    /// What a row on show stands for; `None` for a row that has gone (a
    /// message about it queued before the rows were rebuilt).
    fn kind_of(&self, row: &gtk::ListBoxRow) -> Option<RowKind> {
        self.rows
            .iter()
            .find(|(shown, _)| shown == row)
            .map(|(_, kind)| kind.clone())
    }

    /// Acts on a row people activated.
    fn activate(
        &mut self,
        row: &gtk::ListBoxRow,
        widgets: &TreeComponentWidgets,
        sender: &ComponentSender<Self>,
    ) {
        let output = match self.kind_of(row) {
            Some(RowKind::Entry { selection, .. }) => Some(TreeOutput::Select(selection)),
            Some(RowKind::Heading(id)) => {
                let open = find(&self.tree.assemblies, id).is_some_and(|node| self.is_open(node));
                self.toggled.insert(id, !open);
                self.fill(widgets, sender);
                self.focus_assembly(&widgets.list, id);
                Some(TreeOutput::Select(Selection::Assembly(id)))
            }
            Some(RowKind::Folded(ids)) => {
                for id in &ids {
                    self.toggled.insert(*id, true);
                }
                self.fill(widgets, sender);
                let first = ids.first().copied();
                if let Some(first) = first {
                    self.focus_assembly(&widgets.list, first);
                }
                first.map(|first| TreeOutput::Select(Selection::Assembly(first)))
            }
            Some(RowKind::Joined {
                focus, selection, ..
            }) => match (focus, selection) {
                (Some(target), _) => {
                    self.clicked = Some(Selection::of_target(target));
                    Some(TreeOutput::Focus(target))
                }
                (None, Some(selection)) => Some(TreeOutput::Select(selection)),
                (None, None) => None,
            },
            Some(RowKind::Source(location)) => Some(TreeOutput::Source(location)),
            Some(RowKind::Note) | None => None,
        };
        if let Some(output) = output {
            // Nobody listening is not a mistake: the owner may not want
            // to hear.
            let _ = sender.output(output);
        }
    }
}

#[relm4::component(pub)]
impl Component for TreeComponent {
    type Init = ();
    type Input = TreeInput;
    type Output = TreeOutput;
    type CommandOutput = ();

    view! {
        gtk::Box {
            set_orientation: gtk::Orientation::Vertical,
            add_css_class: "stave-model",

            #[name = "overlay"]
            gtk::Overlay {
                // `External`: the panel may be narrower than its rows ask,
                // so the width watcher below sees the real width and the
                // rows show less.
                #[wrap(Some)]
                set_child = &gtk::ScrolledWindow {
                    set_hscrollbar_policy: gtk::PolicyType::External,
                    set_vexpand: true,

                    #[wrap(Some)]
                    #[name = "list"]
                    set_child = &gtk::ListBox::builder()
                        .accessible_role(gtk::AccessibleRole::Tree)
                        .build() -> gtk::ListBox {
                        set_selection_mode: gtk::SelectionMode::Single,
                        add_css_class: "stave-parts",
                        update_property: &[gtk::accessible::Property::Label(&tr("Parts"))],

                        // A click or Enter chooses a row; the arrow keys
                        // only move the list's selection, which is no
                        // choice.
                        connect_row_activated[sender] => move |_, row| {
                            sender.input_sender().emit(TreeInput::Activated(row.clone()));
                        },
                        // Right, Left, + and − open, fold and walk the
                        // tree; a key that means nothing on the row with
                        // the keyboard is left to GTK.
                        add_controller = gtk::EventControllerKey {
                            connect_key_pressed[sender] => move |keys, key, _, modifiers| {
                                let list = keys.widget().and_downcast::<gtk::ListBox>();
                                match list.and_then(|list| key_step(&list, key, modifiers)) {
                                    Some((row, step)) => {
                                        sender.input_sender().emit(TreeInput::Step { row, step });
                                        gtk::glib::Propagation::Stop
                                    }
                                    None => gtk::glib::Propagation::Proceed,
                                }
                            },
                        },
                    },
                },

                // Watches the panel's width without taking part in its
                // size.
                #[name = "watcher"]
                add_overlay = &gtk::DrawingArea {
                    set_can_target: false,
                    set_can_focus: false,
                    connect_resize[sender] => move |_, width, _| {
                        sender.input_sender().emit(TreeInput::Resized(width));
                    },
                },
            },
        }
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        install_css();
        let model = TreeComponent::default();
        let widgets = view_output!();
        widgets.overlay.set_measure_overlay(&widgets.watcher, false);
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: TreeInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            TreeInput::ShowTree { tree, joints } => {
                self.tree = tree;
                self.joints = joints;
                self.empty = None;
                self.fill(widgets, &sender);
            }
            TreeInput::ShowEmpty(empty) => {
                self.tree = PartsTree::default();
                self.joints = PartJoints::default();
                self.empty = Some(empty);
                self.fill(widgets, &sender);
            }
            TreeInput::SetPlaceholder(placeholder) => {
                if self.placeholder != placeholder {
                    self.placeholder = placeholder;
                    self.fill(widgets, &sender);
                }
            }
            TreeInput::ShowMarks(marks) => {
                self.marks = marks;
                self.fill(widgets, &sender);
            }
            TreeInput::ShowSelection(selection) => {
                // Choosing one of the listed rows keeps them listed under
                // the same part, and then the rows are left as they are,
                // so nothing folds and the list does not move.
                let clicked = self.clicked.take();
                let listing = self
                    .listing
                    .next(selection.as_ref(), &self.joints, clicked.as_ref());
                let was_open = self.open_assemblies();
                let was_listing = self.listing;
                let changed = self.selection != selection;
                self.selection = selection;
                self.listing = listing;
                if changed {
                    self.reveal();
                }
                if listing == was_listing && was_open == self.open_assemblies() {
                    // The same selection again leaves the list's selection
                    // where the keyboard put it.
                    if self.synced != self.selection {
                        self.sync_selection(widgets);
                    }
                } else {
                    // The assembly holding it opens.
                    self.fill(widgets, &sender);
                }
            }
            TreeInput::Activated(row) => self.activate(&row, widgets, &sender),
            TreeInput::Step { row, step } => self.step_row(&row, step, widgets, &sender),
            TreeInput::PartnerClicked(selection) => {
                // A piece of hardware chosen here keeps the rows where
                // they are, as if its row were activated.
                self.clicked = Some(selection.clone());
                let _ = sender.output(TreeOutput::Select(selection));
            }
            TreeInput::Hovered(items) => {
                let _ = sender.output(TreeOutput::Hover(items));
            }
            TreeInput::Resized(width) => {
                let density = Density::for_width(width);
                if self.density != Some(density) {
                    self.density = Some(density);
                    self.fill(widgets, &sender);
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

/// The Parts panel: it owns the component (dropping it shuts the
/// component down), passes its setters on as [`TreeInput`] messages and
/// each [`TreeOutput`] to the callback connected with
/// [`TreeSidebar::connect_outputs`].
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned. Callbacks run from the main
/// loop after the panel's update has finished, never from inside it.
pub struct TreeSidebar {
    /// The panel's content.
    pub root: gtk::Box,
    bridge: Bridge<TreeComponent>,
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
        let bridge = Bridge::<TreeComponent>::launch(());
        TreeSidebar {
            root: bridge.widget().clone(),
            bridge,
        }
    }

    /// Calls `callback` with each [`TreeOutput`] from now on (a row
    /// chosen, a library's item to open, a joint to focus on, a partner
    /// link under the pointer), instead of the one connected before.
    pub fn connect_outputs(&self, callback: impl Fn(TreeOutput) + 'static) {
        self.bridge.connect(callback);
    }

    /// Shows a new tree and its joints, keeping what is folded.
    pub fn show_tree(&self, tree: PartsTree, joints: PartJoints) {
        self.bridge.send(TreeInput::ShowTree { tree, joints });
    }

    /// Shows why there are no parts; for a library, what it defines.
    pub fn show_empty(&self, empty: EmptyState) {
        self.bridge.send(TreeInput::ShowEmpty(empty));
    }

    /// What the tree says while it has no rows: `None` for "Nothing to
    /// show yet".
    pub fn set_placeholder(&self, placeholder: Option<&str>) {
        self.bridge
            .send(TreeInput::SetPlaceholder(placeholder.map(str::to_owned)));
    }

    /// Takes the latest check's marks for the dots.
    pub fn show_marks(&self, marks: Rc<Marks>) {
        self.bridge.send(TreeInput::ShowMarks(marks));
    }

    /// Highlights the selection's row and lists what the selected part is
    /// joined to under it. Choosing one of those rows keeps them listed
    /// under the same part, and then the rows are left as they are, so
    /// nothing folds and the list does not move.
    pub fn show_selection(&self, selection: Option<&Selection>) {
        self.bridge
            .send(TreeInput::ShowSelection(selection.cloned()));
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

impl TreeComponent {
    /// The assemblies whose rows show, in tree order.
    fn open_assemblies(&self) -> Vec<AssemblyId> {
        let mut open = Vec::new();
        let mut stack: Vec<&AssemblyNode> = self.tree.assemblies.iter().rev().collect();
        while let Some(node) = stack.pop() {
            if self.is_open(node) {
                open.push(node.id);
                stack.extend(node.children.iter().rev());
            }
        }
        open
    }

    /// Makes the rows again and puts in the list only those that read
    /// differently from the ones it has: the rows the same at the start
    /// and the end are kept, and only the run between is replaced. The
    /// selection's row is selected again when rows were replaced or the
    /// selection changed; otherwise the list's selection stays where the
    /// keyboard put it.
    fn fill(&mut self, widgets: &TreeComponentWidgets, sender: &ComponentSender<Self>) {
        let list = &widgets.list;
        let density = self.density();
        if density.badge() {
            list.add_css_class("narrow");
        } else {
            list.remove_css_class("narrow");
        }
        let rows = self.rows(density, sender);
        for (row, kind) in &rows {
            row.update_property(&[gtk::accessible::Property::Level(level(kind))]);
            row.add_css_class(kind_class(kind));
        }
        let prints: Vec<String> = rows.iter().map(|(row, kind)| print(row, kind)).collect();
        let mut kept = std::mem::take(&mut self.rows);
        let old_prints = std::mem::take(&mut self.prints);
        let start = old_prints
            .iter()
            .zip(&prints)
            .take_while(|(old, new)| old == new)
            .count();
        let end = old_prints
            .iter()
            .skip(start)
            .rev()
            .zip(prints.iter().skip(start).rev())
            .take_while(|(old, new)| old == new)
            .count();
        let suffix = kept.split_off(kept.len().saturating_sub(end));
        let removed = kept.split_off(start.min(kept.len()));
        let replaced = !removed.is_empty() || rows.len() > start.saturating_add(end);
        for (row, _) in removed {
            list.remove(&row);
        }
        let middle = rows.len().saturating_sub(start).saturating_sub(end);
        for (offset, (row, kind)) in rows.into_iter().skip(start).take(middle).enumerate() {
            let position = start
                .checked_add(offset)
                .and_then(|position| i32::try_from(position).ok())
                .unwrap_or(-1);
            list.insert(&row, position);
            kept.push((row, kind));
        }
        kept.extend(suffix);
        self.rows = kept;
        self.prints = prints;
        if replaced || self.synced != self.selection {
            self.sync_selection(widgets);
        }
        // A link under the pointer may have gone with the old rows without
        // saying it was left.
        if replaced {
            let _ = sender.output(TreeOutput::Hover(Vec::new()));
        }
    }

    /// Opens the assemblies the user folded that hold a part (or what a
    /// part carries) chosen elsewhere, so its row shows. An assembly
    /// chosen itself (its heading, a folded row) is left as the user left
    /// it.
    fn reveal(&mut self) {
        let selection = self.selection.clone().filter(|s| s.item().is_some());
        let under = self.listing.under.map(Selection::of_item);
        let mut stack: Vec<&AssemblyNode> = self.tree.assemblies.iter().collect();
        let mut opened = Vec::new();
        while let Some(node) = stack.pop() {
            let folded = self.toggled.get(&node.id) == Some(&false);
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
            self.toggled.insert(id, true);
        }
    }

    /// Does `step` on `row` (the row with the keyboard): opens or folds
    /// it, or gives the keyboard to the row it leads into or is under.
    /// None of it is a choice. A row that has gone (the key pressed
    /// before the rows were made again) is let go.
    fn step_row(
        &mut self,
        row: &gtk::ListBoxRow,
        step: Step,
        widgets: &TreeComponentWidgets,
        sender: &ComponentSender<Self>,
    ) {
        let Some(index) = self.rows.iter().position(|(shown, _)| shown == row) else {
            return;
        };
        // The nearest row above this one that `wanted` picks.
        let above = |wanted: &dyn Fn(&RowKind) -> bool| {
            self.rows
                .iter()
                .take(index)
                .rev()
                .find(|(_, kind)| wanted(kind))
                .map(|(row, _)| row.clone())
        };
        let kind = self.rows.get(index).map(|(_, kind)| kind.clone());
        match (kind, step) {
            (Some(RowKind::Heading(_)), Step::Open) => {
                let next = self
                    .rows
                    .get(index.saturating_add(1))
                    .map(|(row, _)| row.clone());
                if let Some(next) = next {
                    next.grab_focus();
                }
            }
            (Some(RowKind::Heading(id)), Step::Shut) => {
                self.toggled.insert(id, false);
                self.fill(widgets, sender);
                self.focus_assembly(&widgets.list, id);
            }
            (Some(RowKind::Folded(ids)), Step::Open) => {
                for id in &ids {
                    self.toggled.insert(*id, true);
                }
                self.fill(widgets, sender);
                if let Some(first) = ids.first() {
                    self.focus_assembly(&widgets.list, *first);
                }
            }
            (Some(RowKind::Entry { .. }), Step::Shut) => {
                if let Some(heading) = above(&|kind| matches!(kind, RowKind::Heading(_))) {
                    heading.grab_focus();
                }
            }
            (Some(RowKind::Joined { .. }), Step::Shut) => {
                if let Some(entry) = above(&|kind| matches!(kind, RowKind::Entry { .. })) {
                    entry.grab_focus();
                }
            }
            (
                Some(RowKind::Folded(_) | RowKind::Entry { .. } | RowKind::Joined { .. }),
                Step::Shut | Step::Open,
            )
            | (Some(RowKind::Note | RowKind::Source(_)), Step::Shut | Step::Open)
            | (None, Step::Shut | Step::Open) => {}
        }
    }

    /// Gives the keyboard to an assembly's heading or the folded row it
    /// is in, and selects that row.
    fn focus_assembly(&self, list: &gtk::ListBox, id: AssemblyId) {
        let row = self.rows.iter().find(|(_, kind)| match kind {
            RowKind::Heading(heading) => *heading == id,
            RowKind::Folded(ids) => ids.contains(&id),
            RowKind::Note | RowKind::Entry { .. } | RowKind::Joined { .. } | RowKind::Source(_) => {
                false
            }
        });
        if let Some((row, _)) = row {
            list.select_row(Some(row));
            row.grab_focus();
        }
    }

    /// The rows for the tree as it is now.
    fn rows(
        &self,
        density: Density,
        sender: &ComponentSender<Self>,
    ) -> Vec<(gtk::ListBoxRow, RowKind)> {
        let open =
            |id: AssemblyId| find(&self.tree.assemblies, id).is_some_and(|node| self.is_open(node));
        let blocks = self.joints.blocks(&self.tree, &open);
        let joined: &[JoinedTo] = match self.listing.under {
            Some(Item::Part(part)) => self.joints.joined_to(part),
            Some(Item::Hardware(_) | Item::Member(_)) | None => &[],
        };
        // The parts the selection is joined to, for their rows' blue edge.
        let partners: BTreeSet<Item> = joined
            .iter()
            .flat_map(|row| row.partners.iter().flat_map(|p| p.items.iter().copied()))
            .collect();
        let shown = Shown {
            joints: &self.joints,
            marks: &self.marks,
            under: self.listing.under,
            joined,
            partners: &partners,
            density,
            sender,
        };
        let mut rows = Vec::new();
        for block in &blocks {
            match block {
                Block::Heading { assembly, entries } => {
                    rows.push(heading_row(&self.joints, *assembly, density));
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
            && let Some(empty) = &self.empty
        {
            empty_rows(empty, &mut rows);
        }
        if rows.is_empty() {
            let label = gtk::Label::builder()
                .label(
                    self.placeholder
                        .clone()
                        .unwrap_or_else(|| tr("Nothing to show yet")),
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
        rows
    }

    /// Selects the selection's row (no choice: only activating a row is
    /// one) and remembers the selection it showed.
    fn sync_selection(&mut self, widgets: &TreeComponentWidgets) {
        self.synced = self.selection.clone();
        let selected = self.selection.as_ref().and_then(|selection| {
            let find = |wanted: &dyn Fn(&RowKind) -> bool| {
                self.rows
                    .iter()
                    .find(|(_, kind)| wanted(kind))
                    .map(|(row, _)| row)
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
                    RowKind::Folded(ids) => {
                        ids.iter().any(|id| *selection == Selection::Assembly(*id))
                    }
                    RowKind::Source(location) => {
                        *selection == Selection::Location(location.clone())
                    }
                    RowKind::Note | RowKind::Entry { .. } | RowKind::Joined { .. } => false,
                })
            })
        });
        let list = &widgets.list;
        match selected {
            Some(row) => list.select_row(Some(row)),
            None => list.unselect_all(),
        }
    }
}

/// What Right, Left, + or − asks of the row with the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Open it, or go into it when it is open.
    Open,
    /// Fold it, or go out to the row it is under.
    Shut,
}

/// The row with the keyboard and what a key asks of it, when the key is
/// Right, Left, + or − (no Ctrl, Alt or Super; Right and Left mirrored
/// right to left) and means something on that row. `None` leaves the key
/// to GTK (Right on a joined-to row moves into its partner links). It
/// reads only the list's own widgets: each row's kind is a style class
/// (`kind_class`) and the rows round it are its siblings.
fn key_step(
    list: &gtk::ListBox,
    key: gtk::gdk::Key,
    modifiers: gtk::gdk::ModifierType,
) -> Option<(gtk::ListBoxRow, Step)> {
    use gtk::gdk::{Key, ModifierType};
    if modifiers
        .intersects(ModifierType::CONTROL_MASK | ModifierType::ALT_MASK | ModifierType::SUPER_MASK)
    {
        return None;
    }
    let (right, left) = if list.direction() == gtk::TextDirection::Rtl {
        (Step::Shut, Step::Open)
    } else {
        (Step::Open, Step::Shut)
    };
    let step = match key {
        Key::Right | Key::KP_Right => right,
        Key::Left | Key::KP_Left => left,
        Key::plus | Key::KP_Add => Step::Open,
        Key::minus | Key::KP_Subtract => Step::Shut,
        _ => return None,
    };
    // The row holding the widget with the keyboard (the row itself, or a
    // partner link in it).
    let focused = list.root().and_then(|root| root.focus())?;
    let row = std::iter::successors(Some(focused), gtk::Widget::parent)
        .take_while(|widget| widget != list.upcast_ref::<gtk::Widget>())
        .filter(|widget| widget.parent().as_ref() == Some(list.upcast_ref()))
        .find_map(|widget| widget.downcast::<gtk::ListBoxRow>().ok())?;
    step_means_something(&row, step).then_some((row, step))
}

/// Whether `step` does anything on `row`, as [`TreeComponent::step_row`]
/// does it: on a heading, Open goes to the row after it and Shut folds
/// it; on a folded row, Open opens it; on a part row, Shut goes to the
/// heading above; on a joined-to row, to the part row above.
fn step_means_something(row: &gtk::ListBoxRow, step: Step) -> bool {
    let above = |class: &str| {
        std::iter::successors(row.prev_sibling(), gtk::Widget::prev_sibling)
            .any(|widget| widget.has_css_class(class))
    };
    match step {
        Step::Open => {
            (row.has_css_class(HEADING) && row.next_sibling().is_some())
                || row.has_css_class(FOLDED)
        }
        Step::Shut => {
            row.has_css_class(HEADING)
                || (row.has_css_class(ENTRY) && above(HEADING))
                || (row.has_css_class(JOINED) && above(ENTRY))
        }
    }
}

/// The style classes saying what kind of row a row is, so the key
/// handler can tell without the model.
const HEADING: &str = "stave-tree-heading";
const FOLDED: &str = "stave-tree-folded";
const ENTRY: &str = "stave-tree-entry";
const JOINED: &str = "stave-tree-joined";

/// The style class for a row's kind.
fn kind_class(kind: &RowKind) -> &'static str {
    match kind {
        RowKind::Note => "stave-tree-note",
        RowKind::Heading(_) => HEADING,
        RowKind::Entry { .. } => ENTRY,
        RowKind::Folded(_) => FOLDED,
        RowKind::Joined { .. } => JOINED,
        RowKind::Source(_) => "stave-tree-source",
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
        Mark::Error => tr("has an error"),
        Mark::Warning => tr("has a warning"),
        Mark::Involved => tr("involved in a problem"),
    });
    [detail, mark.as_deref()]
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

/// Appends what `widget` and everything in it show to `out`.
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

/// What the rows need besides the tree itself.
struct Shown<'a> {
    joints: &'a PartJoints,
    marks: &'a Marks,
    /// The part whose joined-to rows show.
    under: Option<Item>,
    joined: &'a [JoinedTo],
    partners: &'a BTreeSet<Item>,
    density: Density,
    /// Where the partner links send what people do.
    sender: &'a ComponentSender<TreeComponent>,
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
        rows.push(plain_heading(&group.heading()));
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
            row.set_tooltip_text(Some(
                &item.summary.clone().unwrap_or_else(|| tr("Open its text")),
            ));
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
            .css_classes(["stave-caps"])
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
            row.set_tooltip_text(Some(&tr("Fold it")));
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
        partners.append(&partner_link(partner, shown));
    }
    content.append(&partners);
    let row = tree_row().child(&content).build();
    let names: Vec<&str> = joined.partners.iter().map(|p| p.label.as_str()).collect();
    let accessible = format!("{}: {}", joined.words, names.join(", "));
    name_row(&row, &accessible, "");
    if joined.focus.is_some() {
        row.set_tooltip_text(Some(&fill_in(
            &tr("{name}. Activate to focus on it"),
            &[("name", &accessible)],
        )));
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
fn partner_link(partner: &Partner, shown: &Shown<'_>) -> gtk::Widget {
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
        .tooltip_text(fill_in(&tr("Select {name}"), &[("name", &partner.label)]))
        .build();
    let input = shown.sender.input_sender().clone();
    button.connect_clicked(move |_| input.emit(TreeInput::PartnerClicked(selection.clone())));
    let motion = gtk::EventControllerMotion::new();
    let items = partner.items.clone();
    let input = shown.sender.input_sender().clone();
    motion.connect_enter(move |_, _, _| input.emit(TreeInput::Hovered(items.clone())));
    let input = shown.sender.input_sender().clone();
    motion.connect_leave(move |_| input.emit(TreeInput::Hovered(Vec::new())));
    button.add_controller(motion);
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
    let row = tree_row()
        .child(&content)
        .tooltip_text(tr("Open it"))
        .build();
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
    use std::cell::RefCell;
    use std::path::Path;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};
    use stave_model::solved::SolvedDesign;

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
            .hardware()
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
        on_gtk_thread(tree_selects_rows_quietly);
    }

    /// Showing the selection selects its row, which is no choice; moving
    /// the list's selection (as the arrow keys do) is none either; a row
    /// activated by hand (a click, Enter) is passed on, once.
    fn tree_selects_rows_quietly() {
        let design = stool();
        let sidebar = TreeSidebar::new();
        let list = list_of(&sidebar);
        let chosen = Rc::new(RefCell::new(Vec::new()));
        sidebar.connect_outputs({
            let chosen = Rc::clone(&chosen);
            move |output| {
                if let TreeOutput::Select(selection) = output {
                    chosen.borrow_mut().push(selection);
                }
            }
        });
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            stave_app::part_joints::PartJoints::new(&design),
        );
        let part = design.parts().first().expect("a part");
        let selection = Selection::of_item(Item::Part(part.id));
        sidebar.show_selection(Some(&selection));
        settle();
        assert!(list.selected_row().is_some(), "the part's row is selected");
        assert!(
            chosen.borrow().is_empty(),
            "the tree's own change is no choice"
        );
        // Showing it again leaves the rows as they are and syncs only.
        sidebar.show_selection(Some(&selection));
        settle();
        assert!(chosen.borrow().is_empty());
        let other = sidebar
            .bridge
            .model()
            .rows
            .iter()
            .find(|(row, kind)| {
                matches!(kind, RowKind::Entry { .. }) && Some(row) != list.selected_row().as_ref()
            })
            .map(|(row, _)| row.clone())
            .expect("another part's row");
        // The arrow keys only move the list's selection: that is no
        // choice. Enter or a click (activating the row) is.
        list.select_row(Some(&other));
        settle();
        assert!(
            chosen.borrow().is_empty(),
            "moving through the rows chooses nothing"
        );
        list.emit_by_name::<()>("row-activated", &[&other]);
        settle();
        assert_eq!(
            chosen.borrow().len(),
            1,
            "a row chosen by hand is passed on"
        );
    }

    #[test]
    fn a_dropped_tree_is_freed() {
        on_gtk_thread(tree_is_freed);
    }

    /// The old panel's partner links held its state, which held the rows
    /// the links sit in: a cycle, so a dropped panel was never freed. Now
    /// the links hold only the component's sender, and dropping the panel
    /// shuts the component down when the main loop next runs: the widget,
    /// its rows and links, the model (and the input channel the runtime
    /// held) and the callbacks are all freed.
    fn tree_is_freed() {
        let design = cabinet();
        let sidebar = TreeSidebar::new();
        let token = Rc::new(());
        let held = Rc::clone(&token);
        sidebar.connect_outputs(move |_| {
            let _ = &held;
        });
        let callbacks = Rc::downgrade(&token);
        drop(token);
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            PartJoints::new(&design),
        );
        // side_l lists what it is joined to, with partner links.
        sidebar.show_selection(Some(&Selection::of_item(part(&design, "side_l"))));
        settle();
        let link = descendants(sidebar.root.upcast_ref())
            .into_iter()
            .find(|widget| widget.has_css_class("stave-partner"))
            .expect("side_l's joined-to rows have partner links")
            .downgrade();
        let row = sidebar
            .bridge
            .model()
            .rows
            .first()
            .map(|(row, _)| row.downgrade())
            .expect("the panel has rows");
        let root = sidebar.root.downgrade();
        let input = sidebar.bridge.sender().clone();
        assert!(root.upgrade().is_some(), "the widget lives while held");
        drop(sidebar);
        settle();
        assert!(root.upgrade().is_none(), "the widget is freed");
        assert!(row.upgrade().is_none(), "its rows are freed");
        assert!(link.upgrade().is_none(), "its partner links are freed");
        assert!(
            input.send(TreeInput::ShowSelection(None)).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callbacks.upgrade().is_none(), "the callbacks are freed");
    }

    #[test]
    fn setters_fill_the_rows_without_choosing() {
        on_gtk_thread(setters_do_not_choose);
    }

    /// The window's setters change the panel and are never heard back as
    /// a choice; rebuilding the rows says only that nothing is hovered.
    fn setters_do_not_choose() {
        let design = stool();
        let sidebar = TreeSidebar::new();
        let heard: Rc<RefCell<Vec<TreeOutput>>> = Rc::default();
        let log = Rc::clone(&heard);
        sidebar.connect_outputs(move |output| match output {
            TreeOutput::Select(_) | TreeOutput::Focus(_) | TreeOutput::Source(_) => {
                log.borrow_mut().push(output);
            }
            TreeOutput::Hover(_) => {}
        });
        sidebar.set_placeholder(Some("No parts yet"));
        settle();
        assert_eq!(
            sidebar.bridge.model().placeholder.as_deref(),
            Some("No parts yet")
        );
        let words = descendants(sidebar.root.upcast_ref())
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
            .map(|label| label.label().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            words,
            vec!["No parts yet".to_owned()],
            "the placeholder shows"
        );
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            PartJoints::new(&design),
        );
        sidebar.show_marks(Rc::default());
        let part = design.parts().first().expect("a part");
        sidebar.show_selection(Some(&Selection::of_item(Item::Part(part.id))));
        settle();
        assert!(sidebar.bridge.model().rows.len() > 1, "the parts have rows");
        assert!(heard.borrow().is_empty(), "setters never choose");
    }

    #[test]
    fn a_choice_made_while_handling_one_is_queued_not_re_entered() {
        on_gtk_thread(choices_are_queued);
    }

    /// The old panel called the window from inside GTK's `row-selected`
    /// signal, and the window showed the selection back into the panel
    /// from there, re-entering it (hence its `syncing` flag and
    /// `try_borrow_mut`). Now a choice (activating a row) only queues a
    /// message; the callback runs after the update has returned (reading
    /// the model there would panic if it had not), and what it does to
    /// the panel, showing the selection or GTK activating another row, is
    /// queued too.
    fn choices_are_queued() {
        let design = stool();
        let sidebar = Rc::new(TreeSidebar::new());
        let list = list_of(&sidebar);
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            PartJoints::new(&design),
        );
        settle();
        let entries: Vec<(gtk::ListBoxRow, Selection)> = sidebar
            .bridge
            .model()
            .rows
            .iter()
            .filter_map(|(row, kind)| match kind {
                RowKind::Entry { selection, .. } => Some((row.clone(), selection.clone())),
                RowKind::Note
                | RowKind::Heading(_)
                | RowKind::Folded(_)
                | RowKind::Joined { .. }
                | RowKind::Source(_) => None,
            })
            .collect();
        let [(first_row, first), (second_row, second), ..] = entries.as_slice() else {
            panic!("the stool has two parts' rows");
        };
        let heard: Rc<RefCell<Vec<Selection>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&sidebar);
        let inner = list.clone();
        let then = second_row.clone();
        sidebar.connect_outputs(move |output| {
            let TreeOutput::Select(selection) = output else {
                return;
            };
            let Some(sidebar) = weak.upgrade() else {
                return;
            };
            // Reading the model would panic were its update still running.
            let _ = sidebar.bridge.model().rows.len();
            let first = log.borrow().is_empty();
            log.borrow_mut().push(selection.clone());
            if first {
                // GTK activating another row from in here only queues a
                // message: nothing is heard under us.
                inner.emit_by_name::<()>("row-activated", &[&then]);
                assert_eq!(log.borrow().len(), 1, "the inner choice is queued");
            }
            // As the window does: show what was chosen. Queued too.
            sidebar.show_selection(Some(&selection));
        });

        list.emit_by_name::<()>("row-activated", &[first_row]);
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        assert_eq!(
            sidebar.bridge.model().selection,
            None,
            "the choice is only queued"
        );

        settle();
        assert_eq!(
            *heard.borrow(),
            vec![first.clone(), second.clone()],
            "each choice after its update, the inner one after the first"
        );
        let model = sidebar.bridge.model();
        assert_eq!(
            model.selection.as_ref(),
            Some(second),
            "the inner choice came last"
        );
        let shown = model
            .rows
            .iter()
            .find(|(row, _)| Some(row) == list.selected_row().as_ref())
            .map(|(_, kind)| kind);
        assert!(
            matches!(shown, Some(RowKind::Entry { selection, .. }) if selection == second),
            "the second part's row is selected"
        );
    }

    /// The cabinet's panel, filled.
    fn cabinet_sidebar() -> (TreeSidebar, SolvedDesign) {
        let design = cabinet();
        let sidebar = TreeSidebar::new();
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            PartJoints::new(&design),
        );
        settle();
        (sidebar, design)
    }

    /// The list's rows, in order.
    fn list_rows(list: &gtk::ListBox) -> Vec<gtk::ListBoxRow> {
        (0..).map_while(|index| list.row_at_index(index)).collect()
    }

    /// The rows the model holds, in order.
    fn model_rows(sidebar: &TreeSidebar) -> Vec<gtk::ListBoxRow> {
        sidebar
            .bridge
            .model()
            .rows
            .iter()
            .map(|(row, _)| row.clone())
            .collect()
    }

    #[test]
    fn a_new_result_updates_rows_in_place() {
        on_gtk_thread(rows_update_in_place);
    }

    /// The same tree, joints or marks again keep every row; a selection
    /// that lists a part's joined-to rows replaces only the run around
    /// it. The list is a tree to screen readers.
    fn rows_update_in_place() {
        let (sidebar, design) = cabinet_sidebar();
        let list = list_of(&sidebar);
        assert_eq!(list.accessible_role(), gtk::AccessibleRole::Tree);
        let before = list_rows(&list);
        assert!(before.len() > 4);
        assert!(
            before
                .iter()
                .all(|row| row.accessible_role() == gtk::AccessibleRole::TreeItem)
        );
        assert_eq!(model_rows(&sidebar), before);
        sidebar.show_tree(
            stave_app::inspect::parts_tree(&design),
            PartJoints::new(&design),
        );
        sidebar.show_marks(Rc::new(Marks::default()));
        settle();
        assert_eq!(
            list_rows(&list),
            before,
            "an unchanged tree made rows again"
        );
        let side = part(&design, "side_l");
        sidebar.show_selection(Some(&Selection::of_item(side)));
        settle();
        let after = list_rows(&list);
        assert!(
            after.len() > before.len(),
            "side_l lists what it is joined to"
        );
        let first = before.first().expect("rows");
        let last = before.last().expect("rows");
        assert_eq!(after.first(), Some(first), "the first row was made again");
        assert_eq!(after.last(), Some(last), "the last row was made again");
        assert_eq!(model_rows(&sidebar), after);
    }

    /// Presses `key` on the list as GTK would, with no modifiers: whether
    /// the tree took it.
    fn press(list: &gtk::ListBox, key: gtk::gdk::Key) -> bool {
        use gtk::glib::translate::IntoGlib;
        let keys = list
            .observe_controllers()
            .into_iter()
            .filter_map(Result::ok)
            .find_map(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
            .expect("the list has a key controller");
        keys.emit_by_name::<bool>(
            "key-pressed",
            &[&key.into_glib(), &0u32, &gtk::gdk::ModifierType::empty()],
        )
    }

    #[test]
    fn left_folds_a_heading_and_right_opens_it() {
        on_gtk_thread(keys_fold_and_open);
    }

    /// Left (−) folds an assembly's heading, Right (+) opens the folded
    /// row again, Left on a part row goes to its heading; none of it is a
    /// choice, and a key that means nothing on its row is left to GTK.
    fn keys_fold_and_open() {
        let (sidebar, _design) = cabinet_sidebar();
        let list = list_of(&sidebar);
        let chosen = Rc::new(Cell::new(0));
        sidebar.connect_outputs({
            let chosen = Rc::clone(&chosen);
            move |output| {
                if let TreeOutput::Select(_) = output {
                    chosen.set(chosen.get() + 1);
                }
            }
        });
        let window = gtk::Window::builder().child(&sidebar.root).build();
        window.present();
        settle();
        // The widget with the keyboard (`has_focus` needs the window to
        // be the active one, which a test's window may not be).
        let focus = || gtk::prelude::GtkWindowExt::focus(&window);
        let find_row = |wanted: &dyn Fn(&RowKind) -> bool| {
            sidebar
                .bridge
                .model()
                .rows
                .iter()
                .enumerate()
                .find(|(_, (_, kind))| wanted(kind))
                .map(|(index, (row, kind))| (index, row.clone(), kind.clone()))
        };
        let (index, heading, kind) =
            find_row(&|kind| matches!(kind, RowKind::Heading(_))).expect("an assembly's heading");
        let RowKind::Heading(id) = kind else {
            unreachable!();
        };
        // Left on the part row after it goes up to the heading.
        let (entry, entry_kind) = sidebar
            .bridge
            .model()
            .rows
            .get(index + 1)
            .cloned()
            .expect("a row after the heading");
        assert!(matches!(entry_kind, RowKind::Entry { .. }));
        assert!(entry.grab_focus());
        assert!(
            !press(&list, gtk::gdk::Key::Right),
            "Right on a part row is left to GTK"
        );
        assert!(press(&list, gtk::gdk::Key::Left));
        settle();
        assert_eq!(focus().as_ref(), Some(heading.upcast_ref::<gtk::Widget>()));
        // Left on the heading folds it.
        assert!(press(&list, gtk::gdk::Key::Left));
        settle();
        let (_, folded, _) =
            find_row(&|kind| matches!(kind, RowKind::Folded(ids) if ids.contains(&id)))
                .expect("the assembly folded");
        assert_eq!(
            focus().as_ref(),
            Some(folded.upcast_ref::<gtk::Widget>()),
            "the folded row has the keyboard"
        );
        assert!(find_row(&|kind| matches!(kind, RowKind::Heading(h) if *h == id)).is_none());
        // Right opens it again.
        assert!(press(&list, gtk::gdk::Key::Right));
        settle();
        let (_, heading, _) = find_row(&|kind| matches!(kind, RowKind::Heading(h) if *h == id))
            .expect("the assembly opened");
        assert_eq!(focus().as_ref(), Some(heading.upcast_ref::<gtk::Widget>()));
        assert_eq!(chosen.get(), 0, "opening and folding chose nothing");
        window.destroy();
    }

    /// The stool, solved.
    fn stool() -> SolvedDesign {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/stool");
        stave_app::open(&folder)
            .expect("the stool opens")
            .design
            .expect("the stool solves")
    }

    /// `root` and every widget inside it.
    fn descendants(root: &gtk::Widget) -> Vec<gtk::Widget> {
        let mut found = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(widget) = stack.pop() {
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                stack.push(next);
            }
            found.push(widget);
        }
        found
    }

    /// The panel's list, to choose rows in as people would.
    fn list_of(sidebar: &TreeSidebar) -> gtk::ListBox {
        descendants(sidebar.root.upcast_ref())
            .into_iter()
            .find_map(|widget| widget.downcast::<gtk::ListBox>().ok())
            .expect("the panel has a list")
    }
}
