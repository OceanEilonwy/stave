//! The docked window (docs/superpowers/specs/2026-11-18-docked-window-design.md):
//! a libpanel dock whose every part is a panel: the 3D view (the Model),
//! Parts, Files, Details, Edit, Code, Problems and Output. Each can be
//! dragged to an edge, into the centre (which splits into columns and
//! rows), or into another panel's tabs; closed (all but the 3D view,
//! which has no ×); and brought back.
//!
//! A first launch shows only the 3D view, Parts on the left and Problems
//! along the bottom ([`Layout::default`]): a beginner meets their project,
//! not a dock. The other panels open from the status bar's "Panels"
//! button (Code also from "Edit the Text", Ctrl+E), each into its home on
//! the compact board (H2, [`Layout::board`]): Parts in a frame above
//! Files on the left; the Model in the centre with the Code panel as the
//! next tab of the same frame, its open files shown as tabs beside Model
//! (`tab_strip`); Details with the Edit panel as tabs on the right; and
//! Problems and Output along the bottom. The status bar's "Problems N"
//! and "Output" buttons open the bottom edge on their panel, and its
//! toggles show or hide each edge. A narrow window folds the side edges
//! away ([`Workbench::narrow_edges`]) and unfolds them as it widens.
//!
//! Where the panels are is remembered per user in `~/.config/stave/layout`
//! (never in a `.stave` file), with the spot each of the 3D view's tool
//! groups is docked in (`view_layout::Spots`); [`Layout`] is that record,
//! read leniently so an old or broken file only means the default.
//!
//! The Code panel holds the open files, so a file always opens wherever
//! the Code panel is.
//!
//! The dock is a relm4 component, [`WorkbenchComponent`]: what it
//! remembers (the last layout, the tool groups' spots, a failure to save)
//! is its model, only its update changes that, and the status bar's
//! buttons and the dock's own signals only send [`WorkbenchInput`]
//! messages (a breakpoint folding the edges too). Where each panel is
//! stays in libpanel's widgets, which people drag about; the queries read
//! them there. It also keeps the panels' own tabs (the Code panel's open
//! files, [`OwnTabs`]): it gives every frame its strip of tabs
//! (`tab_strip`) with a copy of them, hands every strip a new copy when
//! they change, and hears a strip's chosen own tab by message. The tool
//! groups' spots are kept here and nowhere else: the floating controls
//! hear them as [`WorkbenchOutput::ToolsChanged`]. [`Workbench`] owns the
//! controller; its outputs go to the owner's input, forwarded when it is
//! made.

use stave_app::i18n::{fill_in, tr, tr_noop};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use adw::prelude::*;
use gtk::{gio, glib};
use panel::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};

use super::tab_strip::{OwnTabs, PageTab, TabStrip, TabStripOutput};
use super::view_layout::{Group, Spot, Spots};

/// One of the window's panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PanelId {
    /// The 3D view (the "Model" tab), its tool groups floating on it.
    View,
    /// The parts tree.
    Parts,
    /// The project's files.
    Files,
    /// What the selection is (the old floating card's pages).
    Details,
    /// The open files and the editor; the files show as tabs in the strip
    /// of whatever frame holds it.
    Code,
    /// The problems, as a list.
    Problems,
    /// The edit in progress: the card for a chosen candidate, a change of
    /// the design, or a hold being set up; how to start one while there
    /// is none.
    Edit,
    /// What the checks said, one line each.
    Output,
}

impl PanelId {
    /// The panels a first launch shows: the 3D view, Parts and Problems.
    pub const FIRST: [PanelId; 3] = [PanelId::View, PanelId::Parts, PanelId::Problems];

    /// Every panel, in the order the menu lists them.
    pub const ALL: [PanelId; 8] = [
        PanelId::View,
        PanelId::Parts,
        PanelId::Files,
        PanelId::Details,
        PanelId::Edit,
        PanelId::Code,
        PanelId::Problems,
        PanelId::Output,
    ];

    /// The word the layout file and the actions use.
    pub fn word(self) -> &'static str {
        match self {
            PanelId::View => "view",
            PanelId::Parts => "parts",
            PanelId::Files => "files",
            PanelId::Details => "details",
            PanelId::Code => "code",
            PanelId::Problems => "problems",
            PanelId::Edit => "edit",
            PanelId::Output => "output",
        }
    }

    /// The panel a word names.
    pub fn from_word(word: &str) -> Option<PanelId> {
        PanelId::ALL.into_iter().find(|id| id.word() == word)
    }

    /// The panel's tab title.
    pub fn title(self) -> String {
        tr(match self {
            PanelId::View => tr_noop("Model"),
            PanelId::Parts => tr_noop("Parts"),
            PanelId::Files => tr_noop("Files"),
            PanelId::Details => tr_noop("Details"),
            PanelId::Code => tr_noop("Code"),
            PanelId::Problems => tr_noop("Problems"),
            PanelId::Edit => tr_noop("Edit"),
            PanelId::Output => tr_noop("Output"),
        })
    }

    /// The panel's tab icon (the Model's is the tab strip's own cube).
    fn icon_name(self) -> &'static str {
        match self {
            PanelId::View => super::tab_strip::MODEL_ICON,
            PanelId::Output => "utilities-terminal-symbolic",
            PanelId::Parts => "view-list-symbolic",
            PanelId::Files => "folder-symbolic",
            PanelId::Details => "dialog-information-symbolic",
            PanelId::Code => "text-x-generic-symbolic",
            PanelId::Problems => "dialog-warning-symbolic",
            PanelId::Edit => "document-edit-symbolic",
        }
    }
}

/// Where in the window a panel can be docked: an edge or the centre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    /// The left edge (in left-to-right languages).
    Start,
    /// The right edge.
    End,
    /// Along the top, under the header.
    Top,
    /// Along the bottom.
    Bottom,
    /// The middle, in columns and rows.
    Centre,
}

impl Side {
    /// Every side.
    pub const ALL: [Side; 5] = [
        Side::Start,
        Side::End,
        Side::Top,
        Side::Bottom,
        Side::Centre,
    ];
    /// The edges, which can be hidden (the centre always shows).
    pub const EDGES: [Side; 4] = [Side::Start, Side::End, Side::Top, Side::Bottom];

    /// The word the layout file and the actions use.
    pub fn word(self) -> &'static str {
        match self {
            Side::Start => "start",
            Side::End => "end",
            Side::Top => "top",
            Side::Bottom => "bottom",
            Side::Centre => "centre",
        }
    }

    /// The side a word names.
    pub fn from_word(word: &str) -> Option<Side> {
        Side::ALL.into_iter().find(|side| side.word() == word)
    }

    /// The side as a menu says where to move a panel.
    fn menu_label(self) -> String {
        tr(match self {
            Side::Start => tr_noop("Move to the Left"),
            Side::End => tr_noop("Move to the Right"),
            Side::Top => tr_noop("Move to the Top"),
            Side::Bottom => tr_noop("Move to the Bottom"),
            Side::Centre => tr_noop("Move to the Middle"),
        })
    }

    fn area(self) -> panel::Area {
        match self {
            Side::Start => panel::Area::Start,
            Side::End => panel::Area::End,
            Side::Top => panel::Area::Top,
            Side::Bottom => panel::Area::Bottom,
            Side::Centre => panel::Area::Center,
        }
    }

    fn of_area(area: panel::Area) -> Option<Side> {
        match area {
            panel::Area::Start => Some(Side::Start),
            panel::Area::End => Some(Side::End),
            panel::Area::Top => Some(Side::Top),
            panel::Area::Bottom => Some(Side::Bottom),
            panel::Area::Center => Some(Side::Centre),
            _ => None,
        }
    }
}

/// Where one panel is: a side, the column and frame there (from 0; at an
/// edge the column is always 0 and the frame counts along it) and the
/// tab in that frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    /// The side.
    pub side: Side,
    /// The column, from 0 (the centre's; 0 at an edge).
    pub column: u32,
    /// The frame in the column or along the edge, from 0.
    pub frame: u32,
    /// The tab in the frame, from 0.
    pub tab: u32,
}

impl Place {
    fn new(side: Side, column: u32, frame: u32, tab: u32) -> Place {
        Place {
            side,
            column,
            frame,
            tab,
        }
    }
}

/// Where the panels are and which edges show: what is remembered between
/// runs. A panel with no place is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Each open panel's place.
    pub places: BTreeMap<PanelId, Place>,
    /// The edges that show.
    pub revealed: BTreeMap<Side, bool>,
    /// The spot each of the 3D view's tool groups is docked in.
    pub tools: Spots,
}

impl Default for Layout {
    /// What a first launch shows: the 3D view in the centre, Parts on the
    /// left and Problems along the bottom, each where the compact board
    /// puts it; every other panel closed until it is asked for; the tool
    /// groups in their usual spots.
    fn default() -> Layout {
        let board = Layout::board();
        Layout {
            places: board
                .places
                .into_iter()
                .filter(|(id, _)| PanelId::FIRST.contains(id))
                .collect(),
            revealed: BTreeMap::from([
                (Side::Start, true),
                (Side::End, false),
                (Side::Top, false),
                (Side::Bottom, true),
            ]),
            tools: board.tools,
        }
    }
}

impl Layout {
    /// The compact board, every panel's home: Parts above Files on the
    /// left, in two frames; the Model in the centre with Code as the next
    /// tab of its frame; Details and Edit as tabs on the right; Problems
    /// and Output at the bottom, which is hidden; the tool groups in
    /// their usual spots. A closed panel with no place of its own opens
    /// where this puts it.
    pub fn board() -> Layout {
        Layout {
            places: BTreeMap::from([
                (PanelId::View, Place::new(Side::Centre, 0, 0, 0)),
                (PanelId::Code, Place::new(Side::Centre, 0, 0, 1)),
                (PanelId::Parts, Place::new(Side::Start, 0, 0, 0)),
                (PanelId::Files, Place::new(Side::Start, 0, 1, 0)),
                (PanelId::Details, Place::new(Side::End, 0, 0, 0)),
                (PanelId::Edit, Place::new(Side::End, 0, 0, 1)),
                (PanelId::Problems, Place::new(Side::Bottom, 0, 0, 0)),
                (PanelId::Output, Place::new(Side::Bottom, 0, 0, 1)),
            ]),
            revealed: BTreeMap::from([
                (Side::Start, true),
                (Side::End, true),
                (Side::Top, false),
                (Side::Bottom, false),
            ]),
            tools: Spots::default(),
        }
    }

    /// The layout as the file holds it: one line per open panel
    /// (`view centre 0 0 0`: side, column, frame, tab), one per closed one
    /// (`closed files`), one per edge (`show end false`) and one per tool
    /// group on the 3D view (`tool camera right`).
    pub fn to_text(&self) -> String {
        let mut text =
            String::from("# Where Stave's panels are. Delete this file to start again.\n");
        for (id, place) in &self.places {
            let _ = writeln!(
                text,
                "{} {} {} {} {}",
                id.word(),
                place.side.word(),
                place.column,
                place.frame,
                place.tab
            );
        }
        for id in PanelId::ALL {
            if !self.places.contains_key(&id) {
                let _ = writeln!(text, "closed {}", id.word());
            }
        }
        for (side, shown) in &self.revealed {
            let _ = writeln!(text, "show {} {shown}", side.word());
        }
        for group in Group::ALL {
            let _ = writeln!(
                text,
                "tool {} {}",
                group.word(),
                self.tools.spot_of(group).word()
            );
        }
        text
    }

    /// Reads a layout file. Lines it doesn't understand are left out (a
    /// line without a column, `code end 0 0`, means column 0); a panel the
    /// file doesn't mention at all (one newer than the file) goes where
    /// the compact board puts it, the Edit panel as a tab beside Details
    /// wherever the file put Details; a file naming no panel gives the
    /// first launch's panels. Tool groups it doesn't place (an older file
    /// names none) stay in their usual spots.
    pub fn from_text(text: &str) -> Layout {
        let mut places = BTreeMap::new();
        let mut closed = Vec::new();
        let mut revealed = Layout::board().revealed;
        let mut tools = Spots::default();
        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            let (id, side, numbers) = match words.as_slice() {
                ["tool", group, spot] => {
                    if let (Some(group), Some(spot)) =
                        (Group::from_word(group), Spot::from_word(spot))
                    {
                        tools.dock(group, spot);
                    }
                    continue;
                }
                ["show", side, shown] => {
                    if let (Some(side), Ok(shown)) = (Side::from_word(side), shown.parse())
                        && side != Side::Centre
                    {
                        revealed.insert(side, shown);
                    }
                    continue;
                }
                ["closed", id] => {
                    closed.extend(PanelId::from_word(id));
                    continue;
                }
                [id, side, frame, tab] => (*id, *side, ["0", *frame, *tab]),
                [id, side, column, frame, tab] => (*id, *side, [*column, *frame, *tab]),
                _ => continue,
            };
            let [Ok(column), Ok(frame), Ok(tab)] = numbers.map(str::parse::<u32>) else {
                continue;
            };
            if let (Some(id), Some(side)) = (PanelId::from_word(id), Side::from_word(side)) {
                let column = if side == Side::Centre { column } else { 0 };
                places.insert(id, Place::new(side, column, frame, tab));
            }
        }
        if places.is_empty() {
            return Layout {
                tools,
                ..Layout::default()
            };
        }
        let beside_details = places.get(&PanelId::Details).map(|details| {
            let last_tab = places
                .values()
                .filter(|place| {
                    place.side == details.side
                        && place.column == details.column
                        && place.frame == details.frame
                })
                .map(|place| place.tab)
                .max()
                .unwrap_or(details.tab);
            Place {
                tab: last_tab.saturating_add(1),
                ..*details
            }
        });
        for (id, place) in Layout::board().places {
            if !closed.contains(&id) {
                let place = match (id, beside_details) {
                    (PanelId::Edit, Some(beside)) => beside,
                    _ => place,
                };
                places.entry(id).or_insert(place);
            }
        }
        Layout {
            places,
            revealed,
            tools,
        }
    }

    /// The panels at a side, frame by frame (keyed by column and frame),
    /// each frame's in tab order.
    fn frames(&self, side: Side) -> BTreeMap<(u32, u32), Vec<PanelId>> {
        let mut at: BTreeMap<(u32, u32), Vec<(u32, PanelId)>> = BTreeMap::new();
        for (id, place) in &self.places {
            if place.side == side {
                at.entry((place.column, place.frame))
                    .or_default()
                    .push((place.tab, *id));
            }
        }
        at.into_iter()
            .map(|(key, mut tabs)| {
                tabs.sort();
                (key, tabs.into_iter().map(|(_, id)| id).collect())
            })
            .collect()
    }
}

/// The dock's skeleton: an empty pane per edge and a grid for the centre.
const DOCK_UI: &str = r#"<interface>
  <object class="PanelDock" id="dock">
    <property name="vexpand">true</property>
    <property name="hexpand">true</property>
    <property name="start-width">272</property>
    <property name="end-width">340</property>
    <property name="bottom-height">220</property>
    <child><object class="PanelGrid" id="grid"></object></child>
    <child type="start"><object class="PanelPaned" id="start"><property name="orientation">vertical</property></object></child>
    <child type="end"><object class="PanelPaned" id="end"><property name="orientation">vertical</property></object></child>
    <child type="top"><object class="PanelPaned" id="top"><property name="orientation">horizontal</property></object></child>
    <child type="bottom"><object class="PanelPaned" id="bottom"><property name="orientation">horizontal</property></object></child>
  </object>
</interface>"#;

/// Where the layout is remembered.
fn layout_path() -> PathBuf {
    glib::user_config_dir().join("stave").join("layout")
}

/// Writes the layout for next time; the error, said in plain words, when
/// it can't be written.
fn write_layout(layout: &Layout) -> Result<(), String> {
    let path = layout_path();
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, layout.to_text()));
    written.map_err(|error| {
        eprintln!("warning: can't write '{}': {error}", path.display());
        fill_in(
            &tr("Stave couldn't save the panel layout for next time: {error}"),
            &[("error", &error)],
        )
    })
}

/// Where the layout comes from and goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remember {
    /// The user's own file: read at the start, written as it changes.
    Yours,
    /// The default layout, and nothing written (screenshots look the same anywhere).
    Nothing,
    /// This file, read once; nothing written.
    From(PathBuf),
}

/// The dock's widgets: the libpanel dock, the centre's grid, the pane at
/// each edge and each panel. Where each panel is lives in these widgets
/// (people drag panels about without telling anyone), so the layout is
/// always read from them, never kept beside them.
#[derive(Clone)]
struct Dockyard {
    dock: panel::Dock,
    grid: panel::Grid,
    paneds: BTreeMap<Side, panel::Paned>,
    panels: BTreeMap<PanelId, panel::Widget>,
}

impl Dockyard {
    /// The dock, with each panel's content in its panel (none placed yet).
    fn build(contents: Vec<(PanelId, gtk::Widget)>) -> anyhow::Result<Dockyard> {
        panel::init();
        // libpanel takes the dock's children the way a UI file gives them:
        // a pane of frames per edge, and the centre's grid.
        let builder = gtk::Builder::new();
        builder.add_from_string(DOCK_UI).map_err(|error| {
            anyhow::anyhow!(fill_in(
                &tr("can't lay out the window's panels: {error}"),
                &[("error", &error)],
            ))
        })?;
        let (Some(dock), Some(grid)) = (
            builder.object::<panel::Dock>("dock"),
            builder.object::<panel::Grid>("grid"),
        ) else {
            anyhow::bail!(tr("can't lay out the window's panels: the dock is missing"));
        };
        let paneds: BTreeMap<Side, panel::Paned> = Side::EDGES
            .into_iter()
            .filter_map(|side| Some((side, builder.object::<panel::Paned>(side.word())?)))
            .collect();

        // Frames libpanel makes itself (the grid's, and one for a panel
        // dropped somewhere empty) are given their tabs by `give_tabs`
        // afterwards, not by a `create-frame` handler: a frame made in Rust
        // is already sunk, libpanel sinks it again when it places it, and
        // that extra reference keeps the frame and every panel in it alive
        // after the window has closed.

        let panels = contents
            .into_iter()
            .map(|(id, content)| {
                let widget = panel::Widget::builder()
                    .title(id.title())
                    .icon_name(id.icon_name())
                    .can_maximize(true)
                    .build();
                widget.set_id(id.word());
                // Placed, never reached into: a panel's empty state fits
                // its frame by its own component (`empty_state::fit`).
                widget.set_child(Some(&content));
                // The tab's menu moves the panel without dragging it.
                let menu = gio::Menu::new();
                for side in Side::ALL {
                    menu.append(
                        Some(&side.menu_label()),
                        Some(&format!("win.move-panel::{}:{}", id.word(), side.word())),
                    );
                }
                widget.set_menu_model(Some(&menu));
                (id, widget)
            })
            .collect();
        Ok(Dockyard {
            dock,
            grid,
            paneds,
            panels,
        })
    }

    /// Puts every panel where `layout` says, closing the ones it leaves
    /// out, and shows the edges it shows.
    fn apply(&self, layout: &Layout) {
        for widget in self.panels.values() {
            detach(widget);
        }
        for (side, paned) in &self.paneds {
            while let Some(child) = paned.nth_child(0) {
                paned.remove(&child);
            }
            for tabs in layout.frames(*side).into_values() {
                let frame = new_frame();
                paned.append(&frame);
                self.fill(&frame, &tabs);
                self.show_first(&frame, &tabs);
            }
        }
        for ((column, row), tabs) in layout.frames(Side::Centre) {
            let frame = self.grid.column(column).row(row);
            self.fill(&frame, &tabs);
            self.show_first(&frame, &tabs);
        }
        for side in Side::EDGES {
            let shown = layout.revealed.get(&side).copied().unwrap_or(false)
                && !layout.frames(side).is_empty();
            self.dock.set_reveal_area(side.area(), shown);
        }
    }

    /// Gives every frame that has none (the workbench's own new frames,
    /// and those libpanel made itself) the slim strip of tabs, showing
    /// the panels' own tabs as `own` has them; a chosen own tab comes back
    /// to `owner`.
    fn give_tabs(&self, own: &OwnTabs, owner: &relm4::Sender<WorkbenchInput>) {
        self.dock.foreach_frame(|frame| {
            let has_tabs = frame.header().is_some_and(|header| header.is::<TabStrip>());
            if !has_tabs {
                give_tabs(frame, own, owner);
            }
        });
    }

    /// Hands every frame's strip the panels' own tabs as they are now.
    fn show_own_tabs(&self, own: &OwnTabs) {
        self.dock.foreach_frame(|frame| {
            if let Some(strip) = frame
                .header()
                .and_then(|header| header.downcast::<TabStrip>().ok())
            {
                strip.show_own_tabs(own.clone());
            }
        });
    }

    /// Adds these panels to a frame, in order.
    fn fill(&self, frame: &panel::Frame, tabs: &[PanelId]) {
        for id in tabs {
            if let Some(widget) = self.panels.get(id) {
                frame.add(widget);
            }
        }
    }

    /// Shows a frame's first tab: a layout opens on Details, not on the
    /// Edit panel beside it.
    fn show_first(&self, frame: &panel::Frame, tabs: &[PanelId]) {
        if let Some(first) = tabs.first().and_then(|id| self.panels.get(id)) {
            frame.set_visible_child(first);
        }
    }

    /// The side a panel is docked at, if it is open.
    fn side_of(&self, id: PanelId) -> Option<Side> {
        self.panels
            .get(&id)
            .and_then(|widget| widget.position())
            .and_then(|position| Side::of_area(position.area()))
    }

    /// `true` when two panels are tabs of the same frame.
    fn share_a_frame(&self, first: PanelId, second: PanelId) -> bool {
        let frame = |id| self.panels.get(&id).and_then(frame_of);
        matches!((frame(first), frame(second)), (Some(a), Some(b)) if a == b)
    }

    /// `true` while a panel is its frame's shown tab.
    fn is_shown(&self, id: PanelId) -> bool {
        self.panels.get(&id).is_some_and(|widget| {
            frame_of(widget)
                .is_some_and(|frame| frame.visible_child().is_some_and(|shown| shown == *widget))
        })
    }

    /// Where the panels are now, with the tool groups in `tools`; an edge
    /// a narrow window folded away (in `folded`) as it was before folding.
    fn layout(&self, tools: &Spots, folded: &BTreeMap<Side, bool>) -> Layout {
        let mut places = BTreeMap::new();
        let mut counts: BTreeMap<Side, u32> = BTreeMap::new();
        let mut record = |frame: &panel::Frame, side: Side, column: u32, index: u32| {
            for tab in 0..frame.n_pages() {
                let id = frame
                    .page(tab)
                    .and_then(|page| PanelId::from_word(&page.id()));
                if let Some(id) = id {
                    places.insert(id, Place::new(side, column, index, tab));
                }
            }
        };
        // At an edge, frames count along it; the centre's say their own
        // column and row.
        self.dock.foreach_frame(|frame| {
            let Some(side) = Side::of_area(frame.position().area()) else {
                return;
            };
            if side == Side::Centre || frame.n_pages() == 0 {
                return;
            }
            let count = counts.entry(side).or_insert(0);
            let index = *count;
            *count = count.saturating_add(1);
            record(frame, side, 0, index);
        });
        self.grid.foreach_frame(|frame| {
            if frame.n_pages() == 0 {
                return;
            }
            let position = frame.position();
            record(frame, Side::Centre, position.column(), position.row());
        });
        let revealed = Side::EDGES
            .into_iter()
            .map(|side| {
                let shown = folded
                    .get(&side)
                    .copied()
                    .unwrap_or_else(|| self.dock.reveals_area(side.area()));
                (side, shown)
            })
            .collect();
        Layout {
            places,
            revealed,
            tools: tools.clone(),
        }
    }

    /// Shows a panel: brings it back where it was in `last` if it was
    /// closed, shows its edge and makes it the frame's current tab.
    fn present(&self, id: PanelId, last: &Layout) {
        let Some(widget) = self.panels.get(&id) else {
            return;
        };
        self.reopen(id, widget, last);
        self.reveal_edge_of(id);
        widget.raise();
    }

    /// Shows the edge a panel is docked at (the centre always shows).
    fn reveal_edge_of(&self, id: PanelId) {
        if let Some(side) = self.side_of(id)
            && side != Side::Centre
        {
            self.dock.set_reveal_area(side.area(), true);
        }
    }

    /// Puts a closed panel back where it was in `last`, or in its home on
    /// the compact board, behind the tab its frame shows.
    fn reopen(&self, id: PanelId, widget: &panel::Widget, last: &Layout) {
        if widget.parent().is_some() {
            return;
        }
        let place = last
            .places
            .get(&id)
            .copied()
            .or_else(|| Layout::board().places.get(&id).copied());
        if let Some(place) = place {
            let frame = self.frame_at(place);
            let shown = frame.visible_child();
            self.fill(&frame, &[id]);
            if let Some(shown) = shown {
                frame.set_visible_child(&shown);
            }
        }
    }

    /// `true` while a panel is open (in a frame, shown or not).
    fn is_open(&self, id: PanelId) -> bool {
        self.panels
            .get(&id)
            .is_some_and(|widget| frame_of(widget).is_some())
    }

    /// Brings a panel to the front of its frame, unless that frame also
    /// holds the 3D View; a closed panel opens where it was in `last`,
    /// showing its edge, but still not over the model.
    fn bring_forward_beside_view(&self, id: PanelId, last: &Layout) {
        let Some(widget) = self.panels.get(&id) else {
            return;
        };
        if !self.is_open(id) {
            self.reopen(id, widget, last);
            self.reveal_edge_of(id);
        }
        let Some(frame) = frame_of(widget) else {
            return;
        };
        let holds_view = self
            .panels
            .get(&PanelId::View)
            .and_then(frame_of)
            .is_some_and(|view_frame| view_frame == frame);
        if !holds_view {
            widget.raise();
        }
    }

    /// Moves a panel to a side: at an edge into its first frame (as a new
    /// tab), in the middle into a new column beside what is there.
    fn move_to(&self, id: PanelId, side: Side) {
        let Some(widget) = self.panels.get(&id) else {
            return;
        };
        detach(widget);
        let column = match side {
            Side::Centre => self.grid.n_columns(),
            Side::Start | Side::End | Side::Top | Side::Bottom => 0,
        };
        let frame = self.frame_at(Place::new(side, column, 0, 0));
        self.fill(&frame, &[id]);
        if side != Side::Centre {
            self.dock.set_reveal_area(side.area(), true);
        }
        widget.raise();
    }

    /// Opens the bottom edge on a panel, or folds it away when that panel
    /// already shows at an edge.
    fn toggle(&self, id: PanelId, last: &Layout) {
        let edge = self
            .side_of(id)
            .filter(|side| *side != Side::Centre && self.dock.reveals_area(side.area()));
        match edge {
            Some(side) if self.is_shown(id) => self.dock.set_reveal_area(side.area(), false),
            Some(_) | None => self.present(id, last),
        }
    }

    /// The frame at a place, made (with any before it) if there aren't
    /// that many.
    fn frame_at(&self, place: Place) -> panel::Frame {
        if place.side == Side::Centre {
            return self.grid.column(place.column).row(place.frame);
        }
        let mut found = None;
        let mut seen = 0;
        self.dock.foreach_frame(|frame| {
            if Side::of_area(frame.position().area()) == Some(place.side) {
                if seen <= place.frame {
                    found = Some(frame.clone());
                }
                seen += 1;
            }
        });
        match found {
            Some(frame) if seen > place.frame => frame,
            _ => {
                let frame = new_frame();
                if let Some(paned) = self.paneds.get(&place.side) {
                    paned.append(&frame);
                }
                frame
            }
        }
    }
}

/// An empty frame. It is given its tabs by [`Dockyard::give_tabs`] at
/// the end of the workbench's update that made it, with the own tabs as
/// they are then.
fn new_frame() -> panel::Frame {
    panel::Frame::new()
}

/// Puts a frame's tabs along its top as a slim strip of flush tabs, as in
/// an IDE, showing the panels' own tabs as `own` has them; a chosen own
/// tab comes back to `owner`.
fn give_tabs(frame: &panel::Frame, own: &OwnTabs, owner: &relm4::Sender<WorkbenchInput>) {
    frame.set_orientation(gtk::Orientation::Vertical);
    let strip = TabStrip::new(
        own.clone(),
        owner,
        |TabStripOutput::Show { panel, index }| WorkbenchInput::OwnTabChosen { panel, index },
    );
    frame.set_header(Some(&strip));
}

/// The frame holding a panel, if it is open.
fn frame_of(widget: &panel::Widget) -> Option<panel::Frame> {
    let mut ancestor = widget.parent();
    while let Some(current) = ancestor {
        if let Ok(frame) = current.clone().downcast::<panel::Frame>() {
            return Some(frame);
        }
        ancestor = current.parent();
    }
    None
}

/// Takes a panel out of whatever frame holds it.
fn detach(widget: &panel::Widget) {
    if let Some(frame) = frame_of(widget) {
        frame.remove(widget);
    }
}

/// A tab-like button on the status bar that opens a panel on the bottom
/// edge.
fn status_tab(title: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(title)
        .tooltip_text(tooltip)
        .css_classes(["flat", "stave-status-tab"])
        .build()
}

/// The dock's own state: whether the layout is written, the layout last
/// applied or saved, whether a failure to save has been told, and the
/// spot each of the 3D view's tool groups is docked in. Where each panel
/// is lives in the dock's widgets ([`Dockyard`]).
#[derive(Debug)]
pub struct WorkbenchComponent {
    /// Whether the layout is written to the user's file.
    remembers: bool,
    /// The layout last applied or saved, to bring a closed panel back
    /// where it was.
    last: Layout,
    /// A failure to write the layout has been told.
    save_failed: bool,
    /// The spot each of the 3D view's tool groups is docked in.
    tools: Spots,
    /// The side edges a narrow window has folded away, each with whether
    /// it showed before: unfolded as it was when the window widens, and
    /// remembered in the layout as it was, not as folded.
    folded: BTreeMap<Side, bool>,
    /// The panels' own tabs (the Code panel's open files), as their
    /// owner last handed them over: every frame's strip has a copy.
    own_tabs: OwnTabs,
}

/// What a workbench is made with.
pub struct WorkbenchInit {
    yard: Dockyard,
    /// The layout to start with.
    layout: Layout,
    /// Whether the layout is written to the user's file.
    remembers: bool,
}

/// What a workbench is told.
#[derive(Debug)]
pub enum WorkbenchInput {
    /// Put every panel where this layout says, closing the ones it leaves
    /// out, and show the edges it shows.
    Apply(Layout),
    /// Narrow the left and right edges while this breakpoint applies, and
    /// fold them away in a window this narrow: the right edge at 860 sp
    /// and less, the left at 600 sp and less.
    NarrowEdges {
        /// The breakpoint.
        breakpoint: adw::Breakpoint,
        /// The widest window it applies to, in sp.
        width: f64,
    },
    /// A narrow window's breakpoint applied: fold these edges away,
    /// remembering whether each showed.
    Fold(Vec<Side>),
    /// That breakpoint stopped applying: unfold these edges, each shown
    /// again if it showed before folding.
    Unfold(Vec<Side>),
    /// Dock a tool group in a spot (the group there takes its old spot)
    /// and remember it.
    DockTool {
        /// The group.
        group: Group,
        /// Where it goes.
        spot: Spot,
    },
    /// How many problems the status bar's Problems button counts.
    ShowProblemCount(usize),
    /// Remember the layout for next time (not while taking screenshots).
    Save,
    /// The window is closing: remember the layout as [`Save`] does, after
    /// everything the workbench was told before, then say so
    /// ([`WorkbenchOutput::SavedForClosing`]) so the window goes on
    /// closing.
    ///
    /// [`Save`]: WorkbenchInput::Save
    SaveForClosing,
    /// The first launch's panels again, the tool groups back in their
    /// spots.
    Reset,
    /// Show a panel: back where it last was if it was closed, its edge
    /// shown, the frame's current tab.
    Present(PanelId),
    /// Bring a panel to the front of its frame, unless that frame holds
    /// the 3D View.
    BringForwardBesideView(PanelId),
    /// Bring a panel to the front of its frame, as
    /// [`BringForwardBesideView`] does, only if it is open: a closed panel
    /// stays closed. Decided here, in order, after everything the
    /// workbench was told before.
    ///
    /// [`BringForwardBesideView`]: WorkbenchInput::BringForwardBesideView
    BringForwardIfOpen(PanelId),
    /// The Edit panel stopped showing a card: if it shares a frame with
    /// Details and is that frame's shown tab, show Details. Decided here,
    /// in order, after everything the workbench was told before.
    EditPanelHidden,
    /// Say this about the selection at the start of the status bar.
    ShowStatus(String),
    /// Move a panel to a side.
    MoveTo {
        /// The panel.
        panel: PanelId,
        /// Where it goes.
        side: Side,
    },
    /// People pressed the status bar's button for a bottom panel: open
    /// the bottom edge on it, or fold it away.
    Toggle(PanelId),
    /// The bottom edge was shown or hidden (by people or by the dock).
    EdgesChanged,
    /// A panel's own tabs are these now (the Code panel's open files):
    /// every frame's strip shows them in place of the panel's one tab.
    ShowOwnTabs {
        /// The panel.
        panel: PanelId,
        /// Its tabs; none shows its one tab again.
        tabs: Vec<PageTab>,
    },
    /// People chose one of a panel's own tabs in a strip (its frame shows
    /// the panel now).
    OwnTabChosen {
        /// The panel.
        panel: PanelId,
        /// Where the tab is among the panel's own tabs.
        index: usize,
    },
}

/// What a workbench tells its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchOutput {
    /// The tool groups moved (a layout applied or reset, or a group
    /// docked): where they are now. Told after every change, in order, so
    /// the last one told is always where they are.
    ToolsChanged(Spots),
    /// The layout couldn't be saved: a line saying why (told once).
    SaveFailed(String),
    /// The layout is saved (or its failure told), as
    /// [`WorkbenchInput::SaveForClosing`] asked: the window may close.
    SavedForClosing,
    /// People chose one of a panel's own tabs: show the panel on it.
    ShowOwnTab {
        /// The panel.
        panel: PanelId,
        /// Where the tab is among the panel's own tabs.
        index: usize,
    },
}

/// The workbench's widgets.
pub struct WorkbenchWidgets {
    yard: Dockyard,
    /// The status bar's start: a line about the selection.
    status: gtk::Label,
    /// The status bar's "Problems N" button: opens the bottom edge on
    /// the Problems panel.
    problems_button: gtk::Button,
    problems_count: gtk::Label,
    /// The status bar's "Output" button.
    output_button: gtk::Button,
}

impl WorkbenchWidgets {
    /// Marks the status bar's Problems or Output button while its panel
    /// shows at the bottom.
    fn show_status_tabs(&self) {
        let yard = &self.yard;
        let bottom = yard.dock.reveals_area(panel::Area::Bottom);
        for (button, id) in [
            (&self.problems_button, PanelId::Problems),
            (&self.output_button, PanelId::Output),
        ] {
            let at_bottom = yard.side_of(id) == Some(Side::Bottom);
            if bottom && at_bottom && yard.is_shown(id) {
                button.add_css_class("shown");
            } else {
                button.remove_css_class("shown");
            }
        }
    }
}

impl WorkbenchComponent {
    /// Saves the layout as the dock shows it now, if it is remembered.
    fn save(&mut self, yard: &Dockyard, sender: &ComponentSender<Self>) {
        let layout = yard.layout(&self.tools, &self.folded);
        let failure = if self.remembers {
            write_layout(&layout).err()
        } else {
            None
        };
        self.saved(layout, failure, sender);
    }

    /// Keeps the layout just saved as the last one, and tells of a
    /// failure to write it, once.
    fn saved(&mut self, layout: Layout, failure: Option<String>, sender: &ComponentSender<Self>) {
        self.last = layout;
        if let Some(failure) = failure
            && !self.save_failed
        {
            self.save_failed = true;
            // Nobody listening is not a mistake.
            let _ = sender.output(WorkbenchOutput::SaveFailed(failure));
        }
    }

    /// Puts the panels and tool groups where `layout` says.
    fn apply(&mut self, yard: &Dockyard, layout: Layout, sender: &ComponentSender<Self>) {
        yard.apply(&layout);
        self.tools = layout.tools.clone();
        self.last = layout;
        let _ = sender.output(WorkbenchOutput::ToolsChanged(self.tools.clone()));
    }
}

impl Component for WorkbenchComponent {
    type Init = WorkbenchInit;
    type Input = WorkbenchInput;
    type Output = WorkbenchOutput;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = WorkbenchWidgets;

    fn init_root() -> gtk::Box {
        gtk::Box::new(gtk::Orientation::Vertical, 0)
    }

    fn init(
        init: WorkbenchInit,
        root: gtk::Box,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let WorkbenchInit {
            yard,
            layout,
            remembers,
        } = init;
        // The status bar's start: a line about the selection.
        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["stave-status-text"])
            .build();
        // The bottom panels folded into the status bar: "Problems N" and
        // "Output" open the bottom edge on their panel (and fold it away
        // again while it shows).
        let problems_count = gtk::Label::builder()
            .label("0")
            .css_classes(["stave-status-count"])
            .build();
        let problems_content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        problems_content.append(&gtk::Label::new(Some(&tr("Problems"))));
        problems_content.append(&problems_count);
        let problems_button = status_tab(&tr("Problems"), &tr("Show the problems"));
        problems_button.set_child(Some(&problems_content));
        let output_button = status_tab(&tr("Output"), &tr("Show what the checks said"));
        for (button, id) in [
            (&problems_button, PanelId::Problems),
            (&output_button, PanelId::Output),
        ] {
            let input = sender.input_sender().clone();
            button.connect_clicked(move |_| input.emit(WorkbenchInput::Toggle(id)));
        }
        let statusbar = panel::Statusbar::new();
        statusbar.add_prefix(0, &problems_button);
        statusbar.add_prefix(1, &output_button);
        statusbar.add_prefix(2, &status);
        // Brings back any panel that was closed, and resets the layout.
        let panels_menu = gio::Menu::new();
        let shows = gio::Menu::new();
        for id in PanelId::ALL {
            shows.append(
                Some(&id.title()),
                Some(&format!("win.show-panel::{}", id.word())),
            );
        }
        panels_menu.append_section(Some(&tr("Show a panel")), &shows);
        let reset = gio::Menu::new();
        reset.append(Some(&tr("Reset Layout")), Some("win.reset-layout"));
        panels_menu.append_section(None, &reset);
        let panels_button = gtk::MenuButton::builder()
            .label(tr("Panels"))
            .menu_model(&panels_menu)
            .direction(gtk::ArrowType::Up)
            .tooltip_text(tr("Show a closed panel, or put them all back"))
            .css_classes(["flat", "stave-panels-button"])
            .build();
        statusbar.add_suffix(-1, &panels_button);
        for (priority, area) in [panel::Area::Start, panel::Area::Bottom, panel::Area::End]
            .into_iter()
            .enumerate()
        {
            let toggle = panel::ToggleButton::new(&yard.dock, area);
            statusbar.add_suffix(i32::try_from(priority).unwrap_or(0), &toggle);
        }
        root.append(&yard.dock);
        root.append(&statusbar);

        yard.apply(&layout);
        let own_tabs = OwnTabs::new();
        yard.give_tabs(&own_tabs, sender.input_sender());
        let input = sender.input_sender().clone();
        yard.dock
            .connect_notify_local(Some("reveal-bottom"), move |_, _| {
                input.emit(WorkbenchInput::EdgesChanged);
            });
        // A panel dropped somewhere new is remembered at once, so closing
        // it later brings it back there.
        let input = sender.input_sender().clone();
        yard.dock
            .connect_panel_drag_end(move |_, _| input.emit(WorkbenchInput::Save));

        let model = WorkbenchComponent {
            remembers,
            tools: layout.tools.clone(),
            last: layout,
            save_failed: false,
            folded: BTreeMap::new(),
            own_tabs,
        };
        let widgets = WorkbenchWidgets {
            yard,
            status,
            problems_button,
            problems_count,
            output_button,
        };
        widgets.show_status_tabs();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut WorkbenchWidgets,
        message: WorkbenchInput,
        sender: ComponentSender<Self>,
        _root: &gtk::Box,
    ) {
        let yard = &widgets.yard;
        match message {
            WorkbenchInput::Apply(layout) => self.apply(yard, layout, &sender),
            WorkbenchInput::NarrowEdges { breakpoint, width } => {
                breakpoint.add_setter(&yard.dock, "start-width", Some(&220.to_value()));
                breakpoint.add_setter(&yard.dock, "end-width", Some(&260.to_value()));
                let sides = folds_at(width);
                if !sides.is_empty() {
                    // The breakpoint's signals only send.
                    let input = sender.input_sender().clone();
                    let folding = sides.clone();
                    breakpoint.connect_apply(move |_| {
                        input.emit(WorkbenchInput::Fold(folding.clone()));
                    });
                    let input = sender.input_sender().clone();
                    breakpoint.connect_unapply(move |_| {
                        input.emit(WorkbenchInput::Unfold(sides.clone()));
                    });
                }
            }
            WorkbenchInput::Fold(sides) => {
                for side in sides {
                    self.folded
                        .entry(side)
                        .or_insert_with(|| yard.dock.reveals_area(side.area()));
                    yard.dock.set_reveal_area(side.area(), false);
                }
            }
            WorkbenchInput::Unfold(sides) => {
                for side in sides {
                    if let Some(shown) = self.folded.remove(&side) {
                        yard.dock.set_reveal_area(side.area(), shown);
                    }
                }
            }
            WorkbenchInput::DockTool { group, spot } => {
                self.tools.dock(group, spot);
                // Nobody listening is not a mistake.
                let _ = sender.output(WorkbenchOutput::ToolsChanged(self.tools.clone()));
                self.save(yard, &sender);
            }
            WorkbenchInput::ShowProblemCount(count) => {
                let label = &widgets.problems_count;
                label.set_label(&count.to_string());
                if count == 0 {
                    label.remove_css_class("some");
                } else {
                    label.add_css_class("some");
                }
            }
            WorkbenchInput::Save => self.save(yard, &sender),
            WorkbenchInput::SaveForClosing => {
                self.save(yard, &sender);
                // Nobody listening is not a mistake.
                let _ = sender.output(WorkbenchOutput::SavedForClosing);
            }
            WorkbenchInput::Reset => {
                self.apply(yard, Layout::default(), &sender);
                self.save(yard, &sender);
            }
            WorkbenchInput::Present(id) => yard.present(id, &self.last),
            WorkbenchInput::BringForwardBesideView(id) => {
                yard.bring_forward_beside_view(id, &self.last);
            }
            WorkbenchInput::BringForwardIfOpen(id) => {
                if yard.is_open(id) {
                    yard.bring_forward_beside_view(id, &self.last);
                }
            }
            WorkbenchInput::EditPanelHidden => {
                if yard.share_a_frame(PanelId::Edit, PanelId::Details)
                    && yard.is_shown(PanelId::Edit)
                {
                    yard.present(PanelId::Details, &self.last);
                }
            }
            WorkbenchInput::ShowStatus(line) => widgets.status.set_label(&line),
            WorkbenchInput::MoveTo { panel, side } => {
                yard.move_to(panel, side);
                self.save(yard, &sender);
            }
            WorkbenchInput::Toggle(id) => yard.toggle(id, &self.last),
            WorkbenchInput::EdgesChanged => {}
            WorkbenchInput::ShowOwnTabs { panel, tabs } => {
                let changed = if tabs.is_empty() {
                    self.own_tabs.remove(&panel).is_some()
                } else if self.own_tabs.get(&panel) == Some(&tabs) {
                    false
                } else {
                    self.own_tabs.insert(panel, tabs);
                    true
                };
                if changed {
                    yard.show_own_tabs(&self.own_tabs);
                }
            }
            WorkbenchInput::OwnTabChosen { panel, index } => {
                // Nobody listening is not a mistake.
                let _ = sender.output(WorkbenchOutput::ShowOwnTab { panel, index });
            }
        }
        // A message may have made a frame (a panel moved or brought back,
        // or dropped somewhere new, which sends `Save`).
        yard.give_tabs(&self.own_tabs, sender.input_sender());
        widgets.show_status_tabs();
    }
}

/// The side edges a window at most `width` sp wide folds away: the right
/// edge (Details) at 860 sp and less, the left (Parts, Files) at 600 sp
/// and less, as libadwaita's split views collapse.
fn folds_at(width: f64) -> Vec<Side> {
    let mut sides = Vec::new();
    if width <= 860.0 {
        sides.push(Side::End);
    }
    if width <= 600.0 {
        sides.push(Side::Start);
    }
    sides
}

/// The dock and its panels: it owns the [`WorkbenchComponent`] (dropping
/// it shuts the component down) and passes its setters on as
/// [`WorkbenchInput`] messages; its [`WorkbenchOutput`]s go to the
/// owner's input.
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned. Anything decided from where
/// the panels are (whether a panel is open, shown, or shares a frame) is
/// decided by the workbench in its own update, in order, as a message
/// ([`WorkbenchInput::BringForwardIfOpen`],
/// [`WorkbenchInput::EditPanelHidden`]): a query from the owner would
/// read the dock before the moves it had queued were made. The queries
/// (`is_open`, `is_shown`, `share_a_frame`, `layout`, `panel`) are for
/// tests.
pub struct Workbench {
    /// The dock above the status bar, for the window's content.
    pub root: gtk::Box,
    /// The dock's widgets, for tests' queries.
    #[cfg(test)]
    yard: Dockyard,
    controller: Controller<WorkbenchComponent>,
}

impl Workbench {
    /// A dock with each panel's content in its panel, laid out as last
    /// time (or as a first launch shows it). Its outputs go to `owner` as
    /// `message(output)`.
    pub fn new<M: 'static>(
        contents: Vec<(PanelId, gtk::Widget)>,
        remember: Remember,
        owner: &relm4::Sender<M>,
        message: impl Fn(WorkbenchOutput) -> M + 'static,
    ) -> anyhow::Result<Workbench> {
        let yard = Dockyard::build(contents)?;
        let read = |path: PathBuf| {
            std::fs::read_to_string(path)
                .map(|text| Layout::from_text(&text))
                .unwrap_or_default()
        };
        let (layout, remembers) = match remember {
            Remember::Yours => (read(layout_path()), true),
            Remember::Nothing => (Layout::default(), false),
            Remember::From(path) => (read(path), false),
        };
        let controller = WorkbenchComponent::builder()
            .launch(WorkbenchInit {
                #[cfg(test)]
                yard: yard.clone(),
                #[cfg(not(test))]
                yard,
                layout,
                remembers,
            })
            .forward(owner, message);
        Ok(Workbench {
            root: controller.widget().clone(),
            #[cfg(test)]
            yard,
            controller,
        })
    }

    /// Queues `input`; once the workbench has shut down there is nobody
    /// to tell.
    fn send(&self, input: WorkbenchInput) {
        let _ = self.controller.sender().send(input);
    }

    /// A panel's own tabs are these now (the Code panel's open files):
    /// every frame's strip shows them in place of the panel's one tab;
    /// none shows its one tab again.
    pub fn show_own_tabs(&self, panel: PanelId, tabs: Vec<PageTab>) {
        self.send(WorkbenchInput::ShowOwnTabs { panel, tabs });
    }

    /// Puts every panel where `layout` says, closing the ones it leaves
    /// out, and shows the edges it shows.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "kept for the workbench's API; only tests apply a layout now"
        )
    )]
    pub fn apply(&self, layout: Layout) {
        self.send(WorkbenchInput::Apply(layout));
    }

    /// Narrows the left and right edges while `breakpoint` (a window at
    /// most `width` sp wide) applies, so the Model, the window's main
    /// content, keeps room; narrower still, the edges fold away as
    /// libadwaita's split views collapse: the right edge (Details) at
    /// 860 sp and less, the left (Parts, Files) at 600 sp and less. The
    /// status bar's edge toggles still bring a folded edge back, and the
    /// window widening unfolds each edge that showed before.
    pub fn narrow_edges(&self, breakpoint: &adw::Breakpoint, width: f64) {
        self.send(WorkbenchInput::NarrowEdges {
            breakpoint: breakpoint.clone(),
            width,
        });
    }

    /// Where the 3D view's tool groups are docked, as of the last message
    /// the workbench handled: where the view's floating controls start.
    /// Later moves are told as [`WorkbenchOutput::ToolsChanged`].
    pub fn tool_spots(&self) -> Spots {
        self.controller.model().tools.clone()
    }

    /// Docks a tool group in a spot (the group there takes its old spot)
    /// and remembers it.
    pub fn dock_tool(&self, group: Group, spot: Spot) {
        self.send(WorkbenchInput::DockTool { group, spot });
    }

    /// How many problems the status bar's Problems button counts.
    pub fn show_problem_count(&self, count: usize) {
        self.send(WorkbenchInput::ShowProblemCount(count));
    }

    /// A panel's widget, for tests to find its frame.
    #[cfg(test)]
    pub fn panel(&self, id: PanelId) -> Option<&panel::Widget> {
        self.yard.panels.get(&id)
    }

    /// `true` when two panels are tabs of the same frame, for tests.
    #[cfg(test)]
    pub fn share_a_frame(&self, first: PanelId, second: PanelId) -> bool {
        self.yard.share_a_frame(first, second)
    }

    /// `true` while a panel is its frame's shown tab, for tests.
    #[cfg(test)]
    pub fn is_shown(&self, id: PanelId) -> bool {
        self.yard.is_shown(id)
    }

    /// `true` while a panel is open (in a frame, shown or not), for tests.
    #[cfg(test)]
    pub fn is_open(&self, id: PanelId) -> bool {
        self.yard.is_open(id)
    }

    /// Says this about the selection at the start of the status bar (empty
    /// for nothing).
    pub fn show_status(&self, line: &str) {
        self.send(WorkbenchInput::ShowStatus(line.to_owned()));
    }

    /// Brings a panel forward beside the view, as
    /// [`bring_forward_beside_view`](Self::bring_forward_beside_view)
    /// does, only if it is open when the workbench gets to it.
    pub fn bring_forward_if_open(&self, id: PanelId) {
        self.send(WorkbenchInput::BringForwardIfOpen(id));
    }

    /// The Edit panel stopped showing a card: if it shares a frame with
    /// Details and is that frame's shown tab when the workbench gets to
    /// it, Details comes to the front.
    pub fn edit_panel_hidden(&self) {
        self.send(WorkbenchInput::EditPanelHidden);
    }

    /// Where the panels are now (an edge a narrow window folded away as
    /// it was before folding), for tests; the window never asks: the
    /// workbench saves the layout itself, by message.
    #[cfg(test)]
    pub fn layout(&self) -> Layout {
        // Copied out: no borrow is held across the dock's widget calls.
        let (tools, folded) = {
            let model = self.controller.model();
            (model.tools.clone(), model.folded.clone())
        };
        self.yard.layout(&tools, &folded)
    }

    /// Remembers the layout for next time (not while taking screenshots),
    /// after everything the workbench was told before, then answers
    /// [`WorkbenchOutput::SavedForClosing`]: the window closes on that
    /// answer.
    pub fn save_for_closing(&self) {
        self.send(WorkbenchInput::SaveForClosing);
    }

    /// The first launch's panels again, the tool groups back in their
    /// spots.
    pub fn reset(&self) {
        self.send(WorkbenchInput::Reset);
    }

    /// Shows a panel: brings it back where it last was if it was closed,
    /// shows its edge and makes it the frame's current tab.
    pub fn present(&self, id: PanelId) {
        self.send(WorkbenchInput::Present(id));
    }

    /// Brings a panel to the front of its frame, unless that frame also
    /// holds the 3D View (it would hide the model); a closed panel opens
    /// where it last was, showing its edge, but still not over the model.
    /// An open panel's edge isn't shown if people hid it.
    pub fn bring_forward_beside_view(&self, id: PanelId) {
        self.send(WorkbenchInput::BringForwardBesideView(id));
    }

    /// Moves a panel to a side: at an edge into its first frame (as a new
    /// tab), in the middle into a new column beside what is there.
    pub fn move_to(&self, id: PanelId, side: Side) {
        self.send(WorkbenchInput::MoveTo { panel: id, side });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{handle_each, on_gtk_thread, recorder, settle};

    /// A workbench of `contents`, nothing written, whose outputs go to
    /// `owner`.
    fn workbench_for(
        contents: Vec<(PanelId, gtk::Widget)>,
        owner: &relm4::Sender<WorkbenchOutput>,
    ) -> anyhow::Result<Workbench> {
        Workbench::new(contents, Remember::Nothing, owner, std::convert::identity)
    }

    /// A workbench of `contents`, nothing written, whose outputs nobody
    /// hears.
    fn unheard(contents: Vec<(PanelId, gtk::Widget)>) -> anyhow::Result<Workbench> {
        let (owner, _) = recorder();
        workbench_for(contents, &owner)
    }

    #[test]
    fn a_first_launch_shows_the_model_parts_and_problems() {
        let layout = Layout::default();
        assert_eq!(
            layout.places.keys().copied().collect::<Vec<_>>(),
            vec![PanelId::View, PanelId::Parts, PanelId::Problems]
        );
        // Each where the compact board puts it.
        for (id, place) in &layout.places {
            assert_eq!(Layout::board().places.get(id), Some(place));
        }
        assert_eq!(layout.revealed.get(&Side::Start), Some(&true));
        assert_eq!(layout.revealed.get(&Side::Bottom), Some(&true));
        assert_eq!(layout.tools, Spots::default());
        // The rest are written as closed, and read back closed.
        let text = layout.to_text();
        assert!(text.contains("closed code\n"));
        assert!(text.contains("closed details\n"));
        assert_eq!(Layout::from_text(&text), layout);
    }

    #[test]
    fn the_board_is_the_compact_board() {
        let layout = Layout::board();
        // Parts above Files, each in its own frame.
        assert_eq!(
            layout.frames(Side::Start).into_values().collect::<Vec<_>>(),
            vec![vec![PanelId::Parts], vec![PanelId::Files]]
        );
        // The Model, then Code, in one frame: open files sit beside it.
        assert_eq!(
            layout.frames(Side::Centre),
            BTreeMap::from([((0, 0), vec![PanelId::View, PanelId::Code])])
        );
        assert_eq!(
            layout.frames(Side::End).into_values().collect::<Vec<_>>(),
            vec![vec![PanelId::Details, PanelId::Edit]]
        );
        assert_eq!(
            layout
                .frames(Side::Bottom)
                .into_values()
                .collect::<Vec<_>>(),
            vec![vec![PanelId::Problems, PanelId::Output]]
        );
        assert_eq!(layout.revealed.get(&Side::Start), Some(&true));
        assert_eq!(layout.revealed.get(&Side::End), Some(&true));
        assert_eq!(layout.revealed.get(&Side::Bottom), Some(&false));
        assert_eq!(layout.tools, Spots::default());
        assert!(layout.to_text().contains("tool camera right\n"));
    }

    /// Board B, the default before the compact board, as its file was
    /// written.
    const BOARD_B: &str = "# Where Stave's panels are. Delete this file to start again.
view centre 0 0 0
parts start 0 0 0
files start 0 0 1
details start 0 1 0
code bottom 0 0 0
problems bottom 0 0 1
edit start 0 1 1
show start true
show end false
show top false
show bottom true
";

    #[test]
    fn a_board_b_file_still_reads_as_board_b() {
        let layout = Layout::from_text(BOARD_B);
        assert_eq!(
            layout.frames(Side::Start).into_values().collect::<Vec<_>>(),
            vec![
                vec![PanelId::Parts, PanelId::Files],
                vec![PanelId::Details, PanelId::Edit]
            ]
        );
        assert_eq!(
            layout.frames(Side::Centre),
            BTreeMap::from([((0, 0), vec![PanelId::View])])
        );
        // Output, newer than the file, joins the bottom where it shows.
        assert_eq!(
            layout
                .frames(Side::Bottom)
                .into_values()
                .collect::<Vec<_>>(),
            vec![vec![PanelId::Code, PanelId::Problems, PanelId::Output]]
        );
        assert!(layout.frames(Side::End).is_empty());
        assert_eq!(layout.revealed.get(&Side::End), Some(&false));
        assert_eq!(layout.revealed.get(&Side::Bottom), Some(&true));
        // No tool lines: the groups stay in their usual spots.
        assert_eq!(layout.tools, Spots::default());
    }

    #[test]
    fn the_tool_groups_spots_read_back_as_written() {
        let mut layout = Layout::default();
        layout.tools.dock(Group::Camera, Spot::BottomLeft);
        layout.tools.dock(Group::Scene, Spot::Right);
        let text = layout.to_text();
        assert!(text.contains("tool camera bottom-left\n"));
        assert!(text.contains("tool scene right\n"));
        assert_eq!(Layout::from_text(&text), layout);
        // Unknown groups and spots are left out; the rest still apply.
        let layout = Layout::from_text(
            "view centre 0 0 0\ntool ruler right\ntool camera middle\ntool motion top-left\n",
        );
        assert_eq!(layout.tools.spot_of(Group::Motion), Spot::TopLeft);
        assert_eq!(layout.tools.spot_of(Group::Scene), Spot::BottomRight);
        assert_eq!(layout.tools.spot_of(Group::Camera), Spot::Right);
    }

    #[test]
    fn a_layout_reads_back_as_written() {
        let mut layout = Layout::default();
        layout
            .places
            .insert(PanelId::Code, Place::new(Side::Centre, 1, 0, 0));
        layout
            .places
            .insert(PanelId::Problems, Place::new(Side::End, 0, 0, 0));
        layout.revealed.insert(Side::End, true);
        assert_eq!(Layout::from_text(&layout.to_text()), layout);
    }

    #[test]
    fn a_line_without_a_column_means_column_0() {
        let layout = Layout::from_text("code end 0 0\nview centre 2 1 0\n");
        assert_eq!(
            layout.places.get(&PanelId::Code),
            Some(&Place::new(Side::End, 0, 0, 0))
        );
        assert_eq!(
            layout.places.get(&PanelId::View),
            Some(&Place::new(Side::Centre, 2, 1, 0))
        );
    }

    #[test]
    fn a_broken_file_gives_the_default() {
        assert_eq!(Layout::from_text(""), Layout::default());
        assert_eq!(Layout::from_text("nonsense\n\u{0}\u{1}"), Layout::default());
    }

    #[test]
    fn lines_naming_unknown_panels_are_left_out() {
        let layout = Layout::from_text("parts start 0 0\nruler end 0 0\ncode end x 0\n");
        assert_eq!(
            layout.places.get(&PanelId::Parts),
            Some(&Place::new(Side::Start, 0, 0, 0))
        );
        // The broken line is left out, so Code goes to its home.
        assert_eq!(
            layout.places.get(&PanelId::Code),
            Layout::board().places.get(&PanelId::Code)
        );
    }

    #[test]
    fn a_closed_panel_stays_closed() {
        let mut layout = Layout::board();
        layout.places.remove(&PanelId::Files);
        let text = layout.to_text();
        assert!(text.contains("closed files"));
        assert_eq!(Layout::from_text(&text), layout);
    }

    #[test]
    fn a_panel_an_older_file_doesnt_mention_goes_where_the_default_puts_it() {
        let layout = Layout::from_text("parts start 0 0\ncode end 0 0\n");
        assert_eq!(
            layout.places.get(&PanelId::View),
            Some(&Place::new(Side::Centre, 0, 0, 0))
        );
        assert_eq!(
            layout.places.get(&PanelId::Code),
            Some(&Place::new(Side::End, 0, 0, 0))
        );
    }

    #[test]
    fn an_older_file_puts_the_edit_panel_beside_details_wherever_details_is() {
        let layout = Layout::from_text("details end 0 0\nproblems end 0 1\n");
        assert_eq!(
            layout.places.get(&PanelId::Edit),
            Some(&Place::new(Side::End, 0, 0, 2))
        );
        // Closed, it stays closed.
        let layout = Layout::from_text("details end 0 0\nclosed edit\n");
        assert_eq!(layout.places.get(&PanelId::Edit), None);
        // Dragged elsewhere, it stays there.
        let layout = Layout::from_text("details start 1 0\nedit end 0 0\nshow end true\n");
        assert_eq!(
            layout.places.get(&PanelId::Edit),
            Some(&Place::new(Side::End, 0, 0, 0))
        );
    }

    #[test]
    fn the_edit_panel_is_docked_like_any_other() {
        on_gtk_thread(edit_panel_docks);
    }

    /// The Edit panel through the same dock as the others: a tab beside
    /// Details by default, which stays in front; brought forward; moved to
    /// the left edge; its place in the layout that is remembered. Also the
    /// Model and Code sharing a frame, the status bar opening the bottom
    /// edge, and a tool group's spot kept with the layout.
    fn edit_panel_docks() {
        let contents = PanelId::ALL
            .into_iter()
            .map(|id| {
                (
                    id,
                    gtk::Label::new(Some(&id.title())).upcast::<gtk::Widget>(),
                )
            })
            .collect();
        let workbench = unheard(contents).unwrap();
        workbench.apply(Layout::board());
        let window = gtk::Window::builder()
            .default_width(1200)
            .default_height(800)
            .child(&workbench.root)
            .build();
        window.present();
        let context = glib::MainContext::default();
        for _ in 0..50 {
            context.iteration(false);
        }
        settle();
        let layout = workbench.layout();
        assert_eq!(layout, Layout::board());
        assert_eq!(
            layout.places.get(&PanelId::Edit),
            Some(&Place::new(Side::End, 0, 0, 1))
        );
        assert!(workbench.share_a_frame(PanelId::Edit, PanelId::Details));
        assert!(workbench.is_shown(PanelId::Details), "Details shows first");
        assert!(!workbench.is_shown(PanelId::Edit));
        // The Model shows first in its frame; a file chosen elsewhere
        // doesn't cover it, a file opened does.
        assert!(workbench.share_a_frame(PanelId::View, PanelId::Code));
        assert!(workbench.is_shown(PanelId::View));
        workbench.bring_forward_beside_view(PanelId::Code);
        settle();
        assert!(workbench.is_shown(PanelId::View));
        workbench.present(PanelId::Code);
        settle();
        assert!(workbench.is_shown(PanelId::Code));
        workbench.present(PanelId::View);
        settle();
        // The status bar's Problems button opens the hidden bottom edge
        // on Problems, and folds it away again.
        let dock = workbench.yard.dock.clone();
        let toggle = |id| {
            workbench.send(WorkbenchInput::Toggle(id));
            settle();
        };
        assert!(!dock.reveals_area(panel::Area::Bottom));
        toggle(PanelId::Problems);
        assert!(dock.reveals_area(panel::Area::Bottom));
        assert!(workbench.is_shown(PanelId::Problems));
        toggle(PanelId::Output);
        assert!(workbench.is_shown(PanelId::Output));
        toggle(PanelId::Output);
        assert!(!dock.reveals_area(panel::Area::Bottom));
        // An edit starts: its tab comes to the front.
        workbench.bring_forward_beside_view(PanelId::Edit);
        settle();
        assert!(workbench.is_shown(PanelId::Edit));
        // Dragged to the left edge (as its tab's menu moves it).
        workbench.move_to(PanelId::Edit, Side::Start);
        settle();
        let layout = workbench.layout();
        assert_eq!(
            layout.places.get(&PanelId::Edit),
            Some(&Place::new(Side::Start, 0, 0, 1))
        );
        assert_eq!(layout.revealed.get(&Side::Start), Some(&true));
        assert!(!workbench.share_a_frame(PanelId::Edit, PanelId::Details));
        assert!(layout.to_text().contains("edit start 0 0 1"));
        assert_eq!(Layout::from_text(&layout.to_text()), layout);
        // A tool group docked elsewhere is part of the layout.
        workbench.dock_tool(Group::Motion, Spot::TopLeft);
        settle();
        assert_eq!(
            workbench.layout().tools.spot_of(Group::Motion),
            Spot::TopLeft
        );
        workbench.reset();
        settle();
        assert_eq!(workbench.layout().tools, Spots::default());
        assert_eq!(workbench.layout(), Layout::default());
        workbench.move_to(PanelId::Edit, Side::Start);
        // Beside the 3D View, it isn't brought over the model.
        workbench.move_to(PanelId::View, Side::Start);
        workbench.present(PanelId::View);
        workbench.bring_forward_beside_view(PanelId::Edit);
        settle();
        assert!(workbench.is_shown(PanelId::View));
        window.destroy();
    }

    /// Every panel's content a label with its title.
    fn labels() -> Vec<(PanelId, gtk::Widget)> {
        PanelId::ALL
            .into_iter()
            .map(|id| (id, gtk::Label::new(Some(&id.title())).upcast()))
            .collect()
    }

    #[test]
    fn a_dropped_workbench_is_freed() {
        on_gtk_thread(workbench_is_freed);
    }

    /// Dropping the workbench shuts its component down when the main loop
    /// next runs: the dock, its strips of tabs, the model and the message
    /// maker are all freed.
    fn workbench_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let (owner, _) = recorder();
        let Ok(workbench) = Workbench::new(labels(), Remember::Nothing, &owner, move |output| {
            let _ = &held;
            output
        }) else {
            panic!("the dock is built");
        };
        let callback = Rc::downgrade(&token);
        drop(token);
        let root = workbench.root.downgrade();
        let dock = workbench.yard.dock.downgrade();
        let mut strips = Vec::new();
        workbench.yard.dock.foreach_frame(|frame| {
            if let Some(strip) = frame.header() {
                strips.push(strip.downgrade());
            }
        });
        assert!(!strips.is_empty(), "every frame has its strip");
        let input = workbench.controller.sender().clone();
        settle();
        assert!(root.upgrade().is_some(), "the dock lives while held");
        drop(workbench);
        settle();
        assert!(root.upgrade().is_none(), "the root is freed");
        assert!(dock.upgrade().is_none(), "the dock is freed");
        assert!(
            strips.iter().all(|strip| strip.upgrade().is_none()),
            "the strips are freed"
        );
        assert!(
            input.send(WorkbenchInput::Save).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the message maker is freed");
    }

    #[test]
    fn saving_for_closing_answers_after_what_was_told_before() {
        on_gtk_thread(saved_for_closing_comes_last);
    }

    /// Closing asks the workbench to save by message: it saves the layout
    /// as it is after everything queued before (a tool group docked just
    /// before closing is in it) and only then answers, so the window
    /// closes on the answer.
    fn saved_for_closing_comes_last() {
        let (owner, heard) = recorder();
        let Ok(workbench) = workbench_for(labels(), &owner) else {
            panic!("the dock is built");
        };
        settle();
        heard.borrow_mut().clear();
        let mut docked = Spots::default();
        docked.dock(Group::Motion, Spot::TopLeft);
        workbench.dock_tool(Group::Motion, Spot::TopLeft);
        workbench.save_for_closing();
        assert!(heard.borrow().is_empty(), "only queued so far");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                WorkbenchOutput::ToolsChanged(docked.clone()),
                WorkbenchOutput::SavedForClosing,
            ],
            "the answer comes after the docking it saved"
        );
        assert_eq!(workbench.controller.model().last.tools, docked);
    }

    #[test]
    fn setters_change_the_dock_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    /// Presenting, moving and counting change the dock and the model but
    /// tell nobody; only the tool groups moving (a group docked, a layout
    /// applied) is told.
    fn setters_do_not_call_back() {
        let (owner, outputs) = relm4::channel();
        let Ok(workbench) = workbench_for(labels(), &owner) else {
            panic!("the dock is built");
        };
        let heard: Rc<RefCell<Vec<Spots>>> = Rc::default();
        let log = Rc::clone(&heard);
        handle_each(outputs, move |output| {
            if let WorkbenchOutput::ToolsChanged(spots) = output {
                log.borrow_mut().push(spots);
            }
        });
        workbench.show_own_tabs(PanelId::Code, Vec::new());
        workbench.show_problem_count(3);
        workbench.present(PanelId::Code);
        workbench.move_to(PanelId::Files, Side::End);
        settle();
        assert!(workbench.is_shown(PanelId::Code));
        assert_eq!(
            workbench
                .layout()
                .places
                .get(&PanelId::Files)
                .map(|p| p.side),
            Some(Side::End)
        );
        assert_eq!(
            workbench
                .controller
                .model()
                .last
                .places
                .get(&PanelId::Files)
                .map(|p| p.side),
            Some(Side::End),
            "a move is saved as the last layout"
        );
        assert!(heard.borrow().is_empty(), "setters never call back");
        let mut docked = Spots::default();
        docked.dock(Group::Camera, Spot::BottomLeft);
        workbench.dock_tool(Group::Camera, Spot::BottomLeft);
        settle();
        assert_eq!(*heard.borrow(), vec![docked.clone()], "a dock is told");
        let mut moved = Layout::default();
        moved.tools.dock(Group::Scene, Spot::Right);
        workbench.apply(moved.clone());
        settle();
        assert_eq!(*heard.borrow(), vec![docked, moved.tools.clone()]);
        assert_eq!(workbench.tool_spots(), moved.tools);
    }

    #[test]
    fn the_tool_spots_read_after_a_reset_and_a_dock_are_the_latest() {
        on_gtk_thread(tool_spots_follow_reset_then_dock);
    }

    /// A reset and a dock in the same turn of the main loop: each move is
    /// told with where the groups are then, in order, and the last told is
    /// where they end up, as saved.
    fn tool_spots_follow_reset_then_dock() {
        let (owner, outputs) = relm4::channel();
        let Ok(workbench) = workbench_for(labels(), &owner) else {
            panic!("the dock is built");
        };
        let mut start = Layout::default();
        start.tools.dock(Group::Scene, Spot::Right);
        workbench.apply(start);
        settle();
        let heard: Rc<RefCell<Vec<Spots>>> = Rc::default();
        let log = Rc::clone(&heard);
        handle_each(outputs, move |output| {
            if let WorkbenchOutput::ToolsChanged(spots) = output {
                log.borrow_mut().push(spots);
            }
        });
        workbench.reset();
        workbench.dock_tool(Group::Motion, Spot::TopLeft);
        settle();
        let mut expected = Spots::default();
        expected.dock(Group::Motion, Spot::TopLeft);
        assert_eq!(heard.borrow().last(), Some(&expected), "the last told");
        assert_eq!(workbench.layout().tools, expected);
        assert_eq!(
            workbench.controller.model().last.tools,
            expected,
            "as saved"
        );
    }

    #[test]
    fn tool_spots_never_keep_the_workbench_alive() {
        on_gtk_thread(tool_spots_are_weak);
    }

    /// The floating controls start from a copy of the workbench's tool
    /// spots and hear later moves by message: the copy holds nothing of
    /// the workbench, which is freed all the same.
    fn tool_spots_are_weak() {
        let Ok(workbench) = unheard(labels()) else {
            panic!("the dock is built");
        };
        let tools = workbench.tool_spots();
        assert_eq!(tools, Spots::default());
        let root = workbench.root.downgrade();
        drop(workbench);
        settle();
        assert!(root.upgrade().is_none(), "the dock is freed");
        assert_eq!(tools, Spots::default());
    }

    #[test]
    fn a_setter_called_while_handling_an_output_is_queued_not_re_entered() {
        on_gtk_thread(outputs_are_queued);
    }

    /// The old workbench called its tools callback from inside `apply`,
    /// with the callback's own `RefCell` borrowed: a callback that
    /// connected another, or applied a layout, panicked. Now a reset is
    /// only queued; the owner hears of it after the update has returned,
    /// and a panel it presents while it handles that is queued too, as is
    /// the dock's own `notify::reveal-bottom`, fired while the update
    /// reveals the edge.
    fn outputs_are_queued() {
        let (owner, outputs) = relm4::channel();
        let Ok(workbench) = workbench_for(labels(), &owner) else {
            panic!("the dock is built");
        };
        let workbench = Rc::new(workbench);
        // (whether Output showed when the owner heard, and straight after
        // it asked for it)
        let heard: Rc<RefCell<Vec<(bool, bool)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&workbench);
        handle_each(outputs, move |_| {
            let Some(workbench) = weak.upgrade() else {
                return;
            };
            let before = workbench.is_shown(PanelId::Output);
            workbench.present(PanelId::Output);
            log.borrow_mut()
                .push((before, workbench.is_shown(PanelId::Output)));
        });
        workbench.reset();
        assert!(heard.borrow().is_empty(), "nothing has run yet");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![(false, false)],
            "the owner heard after the reset, and its present was queued"
        );
        assert!(workbench.is_shown(PanelId::Output), "then shown");
        assert!(workbench.yard.dock.reveals_area(panel::Area::Bottom));
        workbench.reset();
        settle();
        assert_eq!(heard.borrow().len(), 2, "each reset is heard");
    }

    #[test]
    fn own_tabs_reach_every_strip_and_a_chosen_one_is_told() {
        on_gtk_thread(own_tabs_reach_the_strips);
    }

    /// The Code panel's open files, handed to the workbench, show as tabs
    /// in the strip of the frame that holds the Code panel, also once it
    /// has moved to a frame made later; choosing one is told to the
    /// owner, with the panel and the tab's place.
    fn own_tabs_reach_the_strips() {
        let (owner, heard) = recorder();
        let Ok(workbench) = workbench_for(labels(), &owner) else {
            panic!("the dock is built");
        };
        workbench.apply(Layout::board());
        let files = |count: usize| -> Vec<PageTab> {
            (0..count)
                .map(|index| PageTab {
                    title: format!("file{index}.stave"),
                    tooltip: String::new(),
                    unsaved: false,
                    mark: None,
                    active: index == 0,
                })
                .collect()
        };
        workbench.show_own_tabs(PanelId::Code, files(2));
        settle();
        // The file tabs of the strip heading the Code panel's frame.
        let file_tabs = |workbench: &Workbench| -> Vec<gtk::Button> {
            let Some(strip) = workbench
                .panel(PanelId::Code)
                .and_then(frame_of)
                .and_then(|frame| frame.header())
                .map(|header| header.upcast::<gtk::Widget>())
            else {
                return Vec::new();
            };
            let mut found = Vec::new();
            let mut stack = vec![strip];
            while let Some(widget) = stack.pop() {
                if widget.has_css_class("stave-file-tab")
                    && let Ok(button) = widget.clone().downcast::<gtk::Button>()
                {
                    found.push(button);
                }
                let mut child = widget.last_child();
                while let Some(current) = child {
                    child = current.prev_sibling();
                    stack.push(current);
                }
            }
            found
        };
        assert_eq!(file_tabs(&workbench).len(), 2, "one tab per open file");
        // Moved to a frame made now, its strip shows them too.
        workbench.move_to(PanelId::Code, Side::Centre);
        settle();
        let tabs = file_tabs(&workbench);
        assert_eq!(tabs.len(), 2, "the new frame's strip has them");
        tabs[1].emit_clicked();
        settle();
        assert_eq!(
            heard
                .borrow()
                .iter()
                .filter(|output| matches!(output, WorkbenchOutput::ShowOwnTab { .. }))
                .collect::<Vec<_>>(),
            vec![&WorkbenchOutput::ShowOwnTab {
                panel: PanelId::Code,
                index: 1
            }]
        );
        workbench.show_own_tabs(PanelId::Code, Vec::new());
        settle();
        assert!(
            file_tabs(&workbench).is_empty(),
            "the panel's one tab again"
        );
    }

    #[test]
    fn a_first_launch_opens_the_other_panels_when_asked() {
        on_gtk_thread(first_launch_opens);
    }

    /// The first launch's window: the Model, Parts and Problems open, the
    /// rest closed; choosing a part doesn't open Code; Edit, opened by an
    /// edit, comes to the right edge; Code, asked for, opens beside the
    /// Model and in front.
    fn first_launch_opens() {
        let Ok(workbench) = unheard(labels()) else {
            panic!("the dock is built");
        };
        settle();
        for id in PanelId::ALL {
            assert_eq!(
                workbench.is_open(id),
                PanelId::FIRST.contains(&id),
                "{id:?}"
            );
        }
        let dock = workbench.yard.dock.clone();
        assert!(workbench.is_shown(PanelId::View));
        assert!(workbench.is_shown(PanelId::Problems));
        assert!(dock.reveals_area(panel::Area::Bottom));
        assert!(!dock.reveals_area(panel::Area::End));
        workbench.bring_forward_beside_view(PanelId::Edit);
        settle();
        assert!(workbench.is_open(PanelId::Edit));
        assert!(workbench.is_shown(PanelId::Edit));
        assert!(dock.reveals_area(panel::Area::End));
        workbench.present(PanelId::Code);
        settle();
        assert!(workbench.share_a_frame(PanelId::View, PanelId::Code));
        assert!(workbench.is_shown(PanelId::Code));
        // Closed again, brought forward beside the Model, it doesn't
        // cover the model.
        workbench.reset();
        workbench.bring_forward_beside_view(PanelId::Code);
        settle();
        assert!(workbench.is_open(PanelId::Code));
        assert!(workbench.is_shown(PanelId::View));
    }

    #[test]
    fn decisions_from_where_the_panels_are_follow_the_moves_before_them() {
        on_gtk_thread(decisions_come_in_order);
    }

    /// "Bring Code forward if it is open" and "Edit stopped showing: show
    /// Details if they share a frame" are decided by the workbench in its
    /// own update, so each sees the moves queued before it. Asked of the
    /// dock straight after queueing a move, the answer was the old one.
    fn decisions_come_in_order() {
        let Ok(workbench) = unheard(labels()) else {
            panic!("the dock is built");
        };
        settle();
        // Closed, it stays closed.
        workbench.bring_forward_if_open(PanelId::Code);
        settle();
        assert!(
            !workbench.is_open(PanelId::Code),
            "a closed panel stays closed"
        );
        // Opened by a move queued just before, then hidden behind
        // Problems: the decision sees it open and brings it forward.
        workbench.move_to(PanelId::Code, Side::Bottom);
        workbench.present(PanelId::Problems);
        assert!(
            !workbench.is_open(PanelId::Code),
            "the dock hasn't moved yet: a query here reads it stale"
        );
        workbench.bring_forward_if_open(PanelId::Code);
        settle();
        assert!(workbench.is_open(PanelId::Code));
        assert!(
            workbench.is_shown(PanelId::Code),
            "brought forward in order"
        );
        // Edit shown beside Details by a Present queued just before: when
        // it stops showing a card, Details comes back.
        workbench.apply(Layout::board());
        workbench.present(PanelId::Edit);
        assert!(
            !workbench.share_a_frame(PanelId::Edit, PanelId::Details),
            "the dock hasn't moved yet: a query here reads it stale"
        );
        workbench.edit_panel_hidden();
        settle();
        assert!(workbench.share_a_frame(PanelId::Edit, PanelId::Details));
        assert!(
            workbench.is_shown(PanelId::Details),
            "Details is back in front"
        );
        assert!(!workbench.is_shown(PanelId::Edit));
        // Not shown, Edit leaves its frame as it is.
        workbench.present(PanelId::Parts);
        workbench.edit_panel_hidden();
        settle();
        assert!(workbench.is_shown(PanelId::Details));
        // Moved away from Details, Edit hidden changes nothing.
        workbench.move_to(PanelId::Edit, Side::Start);
        workbench.edit_panel_hidden();
        settle();
        assert!(
            workbench.is_shown(PanelId::Edit),
            "Details isn't in its frame"
        );
    }

    #[test]
    fn a_status_line_is_shown_by_message() {
        on_gtk_thread(status_line);
    }

    /// The status bar's line about the selection is the workbench's own
    /// label, set by message.
    fn status_line() {
        let Ok(workbench) = unheard(labels()) else {
            panic!("the dock is built");
        };
        let label = |workbench: &Workbench| {
            let mut stack = vec![workbench.root.clone().upcast::<gtk::Widget>()];
            while let Some(widget) = stack.pop() {
                if widget.has_css_class("stave-status-text") {
                    return widget.downcast::<gtk::Label>().ok();
                }
                let mut child = widget.first_child();
                while let Some(current) = child {
                    child = current.next_sibling();
                    stack.push(current);
                }
            }
            None
        };
        workbench.show_status("Joins door_l to side_l");
        assert_eq!(
            label(&workbench).map(|label| label.label()).as_deref(),
            Some(""),
            "queued"
        );
        settle();
        assert_eq!(
            label(&workbench).map(|label| label.label()).as_deref(),
            Some("Joins door_l to side_l")
        );
    }

    #[test]
    fn the_centre_is_never_hidden() {
        let layout = Layout::from_text("view centre 0 0 0\nshow centre false\n");
        assert!(!layout.revealed.contains_key(&Side::Centre));
    }

    #[test]
    fn a_narrow_window_folds_the_side_edges() {
        on_gtk_thread(side_edges_fold);
    }

    /// A narrow window folds the side edges away and gives their room to
    /// the Model; widening shows them again; the layout remembers them as
    /// shown all along. The breakpoint's signals only send messages, so
    /// each change shows once the main loop has run.
    fn side_edges_fold() {
        let Ok(workbench) = unheard(labels()) else {
            panic!("the dock is built");
        };
        workbench.apply(Layout::board());
        let condition = |width| {
            adw::BreakpointCondition::new_length(
                adw::BreakpointConditionLengthType::MaxWidth,
                width,
                adw::LengthUnit::Sp,
            )
        };
        let medium = adw::Breakpoint::new(condition(860.0));
        let small = adw::Breakpoint::new(condition(600.0));
        workbench.narrow_edges(&medium, 860.0);
        workbench.narrow_edges(&small, 600.0);
        settle();
        let dock = workbench.yard.dock.clone();
        let shows = |side: Side| dock.reveals_area(side.area());
        assert!(shows(Side::Start) && shows(Side::End));
        medium.emit_by_name::<()>("apply", &[]);
        settle();
        assert!(shows(Side::Start) && !shows(Side::End));
        medium.emit_by_name::<()>("unapply", &[]);
        small.emit_by_name::<()>("apply", &[]);
        settle();
        assert!(!shows(Side::Start) && !shows(Side::End));
        let layout = workbench.layout();
        assert_eq!(layout.revealed.get(&Side::Start), Some(&true));
        assert_eq!(layout.revealed.get(&Side::End), Some(&true));
        small.emit_by_name::<()>("unapply", &[]);
        settle();
        assert!(shows(Side::Start) && shows(Side::End));
    }
}
