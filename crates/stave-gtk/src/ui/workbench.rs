//! The docked window (docs/superpowers/specs/2026-11-18-docked-window-design.md):
//! a libpanel dock whose every part is a panel: the 3D view (the Model),
//! Parts, Files, Details, Edit, Code, Problems and Output. Each can be
//! dragged to an edge, into the centre (which splits into columns and
//! rows), or into another panel's tabs; closed; and brought back.
//!
//! The default layout is the compact board (H2): Parts in a frame above
//! Files on the left; the Model in the centre with the Code panel as the
//! next tab of the same frame, its open files shown as tabs beside Model
//! (`tab_strip`); Details with the Edit panel as tabs on the right; and
//! Problems and Output along the bottom, whose edge starts hidden. The
//! status bar's "Problems N" and "Output" buttons open the bottom edge on
//! their panel, and its toggles show or hide each edge.
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
//! messages. Where each panel is stays in libpanel's widgets, which people
//! drag about; the queries read them there. [`Workbench`] keeps the older
//! API in front of it until the window moves to relm4 too.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use adw::prelude::*;
use gtk::{gio, glib};
use panel::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender};

use super::relm4_shim::Bridge;
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
    pub fn title(self) -> &'static str {
        match self {
            PanelId::View => "Model",
            PanelId::Parts => "Parts",
            PanelId::Files => "Files",
            PanelId::Details => "Details",
            PanelId::Code => "Code",
            PanelId::Problems => "Problems",
            PanelId::Edit => "Edit",
            PanelId::Output => "Output",
        }
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
    fn menu_label(self) -> &'static str {
        match self {
            Side::Start => "Move to the Left",
            Side::End => "Move to the Right",
            Side::Top => "Move to the Top",
            Side::Bottom => "Move to the Bottom",
            Side::Centre => "Move to the Middle",
        }
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
    /// The compact board: Parts above Files on the left, in two frames;
    /// the Model in the centre with Code as the next tab of its frame;
    /// Details and Edit as tabs on the right; Problems and Output at the
    /// bottom, which starts hidden; the tool groups in their usual spots.
    fn default() -> Layout {
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
}

impl Layout {
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
    /// the default puts it, the Edit panel as a tab beside Details
    /// wherever the file put Details; a file naming no panel gives the
    /// default panels. Tool groups it doesn't place (an older file names
    /// none) stay in their usual spots.
    pub fn from_text(text: &str) -> Layout {
        let mut places = BTreeMap::new();
        let mut closed = Vec::new();
        let mut revealed = Layout::default().revealed;
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
        for (id, place) in Layout::default().places {
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
        format!("Stave couldn't save the panel layout for next time: {error}")
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
        builder
            .add_from_string(DOCK_UI)
            .map_err(|error| anyhow::anyhow!("can't lay out the window's panels: {error}"))?;
        let (Some(dock), Some(grid)) = (
            builder.object::<panel::Dock>("dock"),
            builder.object::<panel::Grid>("grid"),
        ) else {
            anyhow::bail!("can't lay out the window's panels: the dock is missing");
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
                widget.set_child(Some(&content));
                // The tab's menu moves the panel without dragging it.
                let menu = gio::Menu::new();
                for side in Side::ALL {
                    menu.append(
                        Some(side.menu_label()),
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
        self.give_tabs();
    }

    /// Gives every frame libpanel made itself the slim strip of tabs the
    /// window's own frames have.
    fn give_tabs(&self) {
        self.dock.foreach_frame(|frame| {
            let has_tabs = frame
                .header()
                .is_some_and(|header| header.is::<super::tab_strip::TabStrip>());
            if !has_tabs {
                give_tabs(frame);
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

    /// Where the panels are now, with the tool groups in `tools`.
    fn layout(&self, tools: &Spots) -> Layout {
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
            .map(|side| (side, self.dock.reveals_area(side.area())))
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
        if widget.parent().is_none() {
            let place = last
                .places
                .get(&id)
                .copied()
                .or_else(|| Layout::default().places.get(&id).copied());
            if let Some(place) = place {
                let frame = self.frame_at(place);
                self.fill(&frame, &[id]);
            }
        }
        if let Some(side) = self.side_of(id)
            && side != Side::Centre
        {
            self.dock.set_reveal_area(side.area(), true);
        }
        widget.raise();
    }

    /// Brings a panel to the front of its frame, unless that frame also
    /// holds the 3D View; a closed panel opens where it was in `last`.
    fn bring_forward_beside_view(&self, id: PanelId, last: &Layout) {
        let Some(widget) = self.panels.get(&id) else {
            return;
        };
        let Some(frame) = frame_of(widget) else {
            self.present(id, last);
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

/// An empty frame, its tabs along the top wherever it is.
fn new_frame() -> panel::Frame {
    let frame = panel::Frame::new();
    give_tabs(&frame);
    frame
}

/// Puts a frame's tabs along its top as a slim strip of flush tabs, as in
/// an IDE.
fn give_tabs(frame: &panel::Frame) {
    frame.set_orientation(gtk::Orientation::Vertical);
    frame.set_header(Some(&super::tab_strip::TabStrip::new()));
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
}

/// What a workbench is made with.
pub struct WorkbenchInit {
    yard: Dockyard,
    /// The status bar's start, for a line about the selection.
    status: gtk::Label,
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
    /// Narrow the left and right edges while this breakpoint applies.
    NarrowEdges(adw::Breakpoint),
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
    /// The layout was just saved, by the shim as the window closes: keep
    /// it as the last one, and tell of the failure, once.
    Saved {
        /// The layout saved.
        layout: Layout,
        /// Why it couldn't be written, if it couldn't.
        failure: Option<String>,
    },
    /// The compact board again, the tool groups back in their spots.
    Reset,
    /// Show a panel: back where it last was if it was closed, its edge
    /// shown, the frame's current tab.
    Present(PanelId),
    /// Bring a panel to the front of its frame, unless that frame holds
    /// the 3D View.
    BringForwardBesideView(PanelId),
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
}

/// The workbench's widgets.
pub struct WorkbenchWidgets {
    yard: Dockyard,
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
        let layout = yard.layout(&self.tools);
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
            status,
            layout,
            remembers,
        } = init;
        // The bottom panels folded into the status bar: "Problems N" and
        // "Output" open the bottom edge on their panel (and fold it away
        // again while it shows).
        let problems_count = gtk::Label::builder()
            .label("0")
            .css_classes(["stave-status-count"])
            .build();
        let problems_content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        problems_content.append(&gtk::Label::new(Some("Problems")));
        problems_content.append(&problems_count);
        let problems_button = status_tab("Problems", "Show the problems");
        problems_button.set_child(Some(&problems_content));
        let output_button = status_tab("Output", "Show what the checks said");
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
                Some(id.title()),
                Some(&format!("win.show-panel::{}", id.word())),
            );
        }
        panels_menu.append_section(Some("Show a panel"), &shows);
        let reset = gio::Menu::new();
        reset.append(Some("Reset Layout"), Some("win.reset-layout"));
        panels_menu.append_section(None, &reset);
        let panels_button = gtk::MenuButton::builder()
            .label("Panels")
            .menu_model(&panels_menu)
            .direction(gtk::ArrowType::Up)
            .tooltip_text("Show a closed panel, or put them all back")
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
        };
        let widgets = WorkbenchWidgets {
            yard,
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
            WorkbenchInput::NarrowEdges(breakpoint) => {
                breakpoint.add_setter(&yard.dock, "start-width", Some(&220.to_value()));
                breakpoint.add_setter(&yard.dock, "end-width", Some(&260.to_value()));
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
            WorkbenchInput::Saved { layout, failure } => self.saved(layout, failure, &sender),
            WorkbenchInput::Reset => {
                self.apply(yard, Layout::default(), &sender);
                self.save(yard, &sender);
            }
            WorkbenchInput::Present(id) => yard.present(id, &self.last),
            WorkbenchInput::BringForwardBesideView(id) => {
                yard.bring_forward_beside_view(id, &self.last);
            }
            WorkbenchInput::MoveTo { panel, side } => {
                yard.move_to(panel, side);
                self.save(yard, &sender);
            }
            WorkbenchInput::Toggle(id) => yard.toggle(id, &self.last),
            WorkbenchInput::EdgesChanged => {}
        }
        // A message may have made a frame (a panel moved or brought back,
        // or dropped somewhere new, which sends `Save`).
        yard.give_tabs();
        widgets.show_status_tabs();
    }
}

/// The dock and its panels: it owns the [`WorkbenchComponent`] (dropping
/// it shuts the component down), passes its setters on as
/// [`WorkbenchInput`] messages and its [`WorkbenchOutput`]s to the
/// callback connected with [`Workbench::connect_outputs`].
///
/// Setters are queued: they take effect when the main loop next runs,
/// after whatever called them has returned. The queries (`layout`,
/// `is_shown`, `share_a_frame`, `panel`) read the dock's widgets as they
/// are now.
pub struct Workbench {
    /// The dock above the status bar, for the window's content.
    pub root: gtk::Box,
    /// The status bar's start, for a line about the selection.
    pub status: gtk::Label,
    /// The dock's widgets, for the queries.
    yard: Dockyard,
    /// Whether the layout is written to the user's file.
    remembers: bool,
    bridge: Bridge<WorkbenchComponent>,
}

impl Workbench {
    /// A dock with each panel's content in its panel, laid out as last
    /// time (or as the compact board).
    pub fn new(
        contents: Vec<(PanelId, gtk::Widget)>,
        remember: Remember,
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
        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["stave-status-text"])
            .build();
        let bridge = Bridge::<WorkbenchComponent>::launch(WorkbenchInit {
            yard: yard.clone(),
            status: status.clone(),
            layout,
            remembers,
        });
        Ok(Workbench {
            root: bridge.widget().clone(),
            status,
            yard,
            remembers,
            bridge,
        })
    }

    /// Calls `callback` with each output from now on (the tool groups
    /// moved, the layout couldn't be saved), instead of the one connected
    /// before.
    pub fn connect_outputs(&self, callback: impl Fn(WorkbenchOutput) + 'static) {
        self.bridge.connect(callback);
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
        self.bridge.send(WorkbenchInput::Apply(layout));
    }

    /// Narrows the left and right edges while `breakpoint` applies, so the
    /// Model keeps room in a narrow window.
    pub fn narrow_edges(&self, breakpoint: &adw::Breakpoint) {
        self.bridge
            .send(WorkbenchInput::NarrowEdges(breakpoint.clone()));
    }

    /// Where the 3D view's tool groups are docked, as of the last message
    /// the workbench handled: where the view's floating controls start.
    /// Later moves are told as [`WorkbenchOutput::ToolsChanged`].
    pub fn tool_spots(&self) -> Spots {
        self.bridge.model().tools.clone()
    }

    /// Docks a tool group in a spot (the group there takes its old spot)
    /// and remembers it.
    pub fn dock_tool(&self, group: Group, spot: Spot) {
        self.bridge.send(WorkbenchInput::DockTool { group, spot });
    }

    /// How many problems the status bar's Problems button counts.
    pub fn show_problem_count(&self, count: usize) {
        self.bridge.send(WorkbenchInput::ShowProblemCount(count));
    }

    /// A panel's widget.
    pub fn panel(&self, id: PanelId) -> Option<&panel::Widget> {
        self.yard.panels.get(&id)
    }

    /// `true` when two panels are tabs of the same frame.
    pub fn share_a_frame(&self, first: PanelId, second: PanelId) -> bool {
        self.yard.share_a_frame(first, second)
    }

    /// `true` while a panel is its frame's shown tab.
    pub fn is_shown(&self, id: PanelId) -> bool {
        self.yard.is_shown(id)
    }

    /// Where the panels are now.
    pub fn layout(&self) -> Layout {
        let tools = self.bridge.model().tools.clone();
        self.yard.layout(&tools)
    }

    /// Remembers the layout for next time (not while taking screenshots).
    ///
    /// Written at once, not queued: the window saves as it closes, and
    /// the main loop may not run again after that. The component hears
    /// what was saved by message.
    pub fn save(&self) {
        let layout = self.layout();
        let failure = if self.remembers {
            write_layout(&layout).err()
        } else {
            None
        };
        self.bridge.send(WorkbenchInput::Saved { layout, failure });
    }

    /// The compact board again, the tool groups back in their spots.
    pub fn reset(&self) {
        self.bridge.send(WorkbenchInput::Reset);
    }

    /// Shows a panel: brings it back where it last was if it was closed,
    /// shows its edge and makes it the frame's current tab.
    pub fn present(&self, id: PanelId) {
        self.bridge.send(WorkbenchInput::Present(id));
    }

    /// Brings a panel to the front of its frame, unless that frame also
    /// holds the 3D View (it would hide the model); a closed panel opens
    /// where it last was. Its edge isn't shown if people hid it.
    pub fn bring_forward_beside_view(&self, id: PanelId) {
        self.bridge.send(WorkbenchInput::BringForwardBesideView(id));
    }

    /// Moves a panel to a side: at an edge into its first frame (as a new
    /// tab), in the middle into a new column beside what is there.
    pub fn move_to(&self, id: PanelId, side: Side) {
        self.bridge.send(WorkbenchInput::MoveTo { panel: id, side });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    #[test]
    fn the_default_is_the_compact_board() {
        let layout = Layout::default();
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
        // The broken line is left out, so Code goes to its default place.
        assert_eq!(
            layout.places.get(&PanelId::Code),
            Layout::default().places.get(&PanelId::Code)
        );
    }

    #[test]
    fn a_closed_panel_stays_closed() {
        let mut layout = Layout::default();
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
                    gtk::Label::new(Some(id.title())).upcast::<gtk::Widget>(),
                )
            })
            .collect();
        let workbench = Workbench::new(contents, Remember::Nothing).unwrap();
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
        assert_eq!(layout, Layout::default());
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
            workbench.bridge.send(WorkbenchInput::Toggle(id));
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
            .map(|id| (id, gtk::Label::new(Some(id.title())).upcast()))
            .collect()
    }

    #[test]
    fn a_dropped_workbench_is_freed() {
        on_gtk_thread(workbench_is_freed);
    }

    /// Dropping the workbench shuts its component down when the main loop
    /// next runs: the dock, the model and the callbacks are all freed.
    fn workbench_is_freed() {
        let Ok(workbench) = Workbench::new(labels(), Remember::Nothing) else {
            panic!("the dock is built");
        };
        let token = Rc::new(());
        let held = Rc::clone(&token);
        workbench.connect_outputs(move |_| {
            let _ = &held;
        });
        let callback = Rc::downgrade(&token);
        drop(token);
        let root = workbench.root.downgrade();
        let dock = workbench.yard.dock.downgrade();
        let input = workbench.bridge.sender().clone();
        settle();
        assert!(root.upgrade().is_some(), "the dock lives while held");
        drop(workbench);
        settle();
        assert!(root.upgrade().is_none(), "the root is freed");
        assert!(dock.upgrade().is_none(), "the dock is freed");
        assert!(
            input.send(WorkbenchInput::Save).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn setters_change_the_dock_without_calling_back() {
        on_gtk_thread(setters_do_not_call_back);
    }

    /// Presenting, moving and counting change the dock and the model but
    /// tell nobody; only the tool groups moving (a group docked, a layout
    /// applied) is told.
    fn setters_do_not_call_back() {
        let Ok(workbench) = Workbench::new(labels(), Remember::Nothing) else {
            panic!("the dock is built");
        };
        let heard: Rc<RefCell<Vec<Spots>>> = Rc::default();
        let log = Rc::clone(&heard);
        workbench.connect_outputs(move |output| {
            if let WorkbenchOutput::ToolsChanged(spots) = output {
                log.borrow_mut().push(spots);
            }
        });
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
                .bridge
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
        let Ok(workbench) = Workbench::new(labels(), Remember::Nothing) else {
            panic!("the dock is built");
        };
        let mut start = Layout::default();
        start.tools.dock(Group::Scene, Spot::Right);
        workbench.apply(start);
        settle();
        let heard: Rc<RefCell<Vec<Spots>>> = Rc::default();
        let log = Rc::clone(&heard);
        workbench.connect_outputs(move |output| {
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
        assert_eq!(workbench.bridge.model().last.tools, expected, "as saved");
    }

    #[test]
    fn tool_spots_never_keep_the_workbench_alive() {
        on_gtk_thread(tool_spots_are_weak);
    }

    /// The floating controls start from a copy of the workbench's tool
    /// spots and hear later moves by message: the copy holds nothing of
    /// the workbench, which is freed all the same.
    fn tool_spots_are_weak() {
        let Ok(workbench) = Workbench::new(labels(), Remember::Nothing) else {
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
    /// only queued; the callback runs after the update has returned, and a
    /// panel presented from inside it is queued too, as is the dock's own
    /// `notify::reveal-bottom`, fired while the update reveals the edge.
    fn outputs_are_queued() {
        let Ok(workbench) = Workbench::new(labels(), Remember::Nothing) else {
            panic!("the dock is built");
        };
        let workbench = Rc::new(workbench);
        // (whether Output showed when the callback was called, and
        // straight after it asked for it)
        let heard: Rc<RefCell<Vec<(bool, bool)>>> = Rc::default();
        let log = Rc::clone(&heard);
        let weak = Rc::downgrade(&workbench);
        workbench.connect_outputs(move |_| {
            let Some(workbench) = weak.upgrade() else {
                return;
            };
            let before = workbench.is_shown(PanelId::Output);
            // Connecting from here is fine: nothing is borrowed.
            let inner = Rc::clone(&log);
            let again = weak.clone();
            workbench.connect_outputs(move |_| {
                if let Some(workbench) = again.upgrade() {
                    inner
                        .borrow_mut()
                        .push((workbench.is_shown(PanelId::Output), true));
                }
            });
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
            "the callback ran after the reset, and its present was queued"
        );
        assert!(workbench.is_shown(PanelId::Output), "then shown");
        assert!(workbench.yard.dock.reveals_area(panel::Area::Bottom));
        workbench.reset();
        settle();
        assert_eq!(heard.borrow().len(), 2, "the callback connected inside");
    }

    #[test]
    fn the_centre_is_never_hidden() {
        let layout = Layout::from_text("view centre 0 0 0\nshow centre false\n");
        assert!(!layout.revealed.contains_key(&Side::Centre));
    }
}
