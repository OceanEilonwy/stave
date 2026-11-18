//! The docked window (docs/superpowers/specs/2026-11-18-docked-window-design.md):
//! a libpanel dock whose every part is a panel: the 3D view, its controls,
//! Parts, Files, Details, Code and Problems. Each can be dragged to an
//! edge, into the centre (which splits into columns and rows), or into
//! another panel's tabs; closed; and brought back. Nothing floats over the
//! model but the problem pins.
//!
//! The default layout is the canvas's board B: Parts (with Files) above
//! Details on the left; the 3D view in the centre with its controls under
//! it; Code and Problems along the bottom; the right edge empty and hidden.
//! Where the panels are is remembered per user in `~/.config/stave/layout`
//! (never in a `.stave` file); [`Layout`] is that record, read leniently so
//! an old or broken file only means the default.
//!
//! The Code panel holds the open files as its own tabs, so a file always
//! opens wherever the Code panel is.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use panel::prelude::*;

/// One of the window's panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PanelId {
    /// The 3D view, with its tools in a strip along its foot (scene, Pull
    /// apart, X-ray, the sliders, a posed problem's swing, focus mode's
    /// steps) and the camera buttons on the view.
    View,
    /// The parts tree.
    Parts,
    /// The project's files.
    Files,
    /// What the selection is (the old floating card's pages).
    Details,
    /// The open files, as tabs, and the editor.
    Code,
    /// The problems, as a list.
    Problems,
}

impl PanelId {
    /// Every panel, in the order the menu lists them.
    pub const ALL: [PanelId; 6] = [
        PanelId::View,
        PanelId::Parts,
        PanelId::Files,
        PanelId::Details,
        PanelId::Code,
        PanelId::Problems,
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
        }
    }

    /// The panel a word names.
    pub fn from_word(word: &str) -> Option<PanelId> {
        PanelId::ALL.into_iter().find(|id| id.word() == word)
    }

    /// The panel's tab title.
    pub fn title(self) -> &'static str {
        match self {
            PanelId::View => "3D View",
            PanelId::Parts => "Parts",
            PanelId::Files => "Files",
            PanelId::Details => "Details",
            PanelId::Code => "Code",
            PanelId::Problems => "Problems",
        }
    }

    /// The panel's tab icon.
    fn icon_name(self) -> &'static str {
        match self {
            PanelId::View => "view-reveal-symbolic",
            PanelId::Parts => "view-list-symbolic",
            PanelId::Files => "folder-symbolic",
            PanelId::Details => "dialog-information-symbolic",
            PanelId::Code => "text-x-generic-symbolic",
            PanelId::Problems => "dialog-warning-symbolic",
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
}

impl Default for Layout {
    /// Board B: Parts and Files above Details on the left, the 3D view in
    /// the centre, Code and Problems at the bottom, nothing on the right.
    fn default() -> Layout {
        Layout {
            places: BTreeMap::from([
                (PanelId::View, Place::new(Side::Centre, 0, 0, 0)),
                (PanelId::Parts, Place::new(Side::Start, 0, 0, 0)),
                (PanelId::Files, Place::new(Side::Start, 0, 0, 1)),
                (PanelId::Details, Place::new(Side::Start, 0, 1, 0)),
                (PanelId::Code, Place::new(Side::Bottom, 0, 0, 0)),
                (PanelId::Problems, Place::new(Side::Bottom, 0, 0, 1)),
            ]),
            revealed: BTreeMap::from([
                (Side::Start, true),
                (Side::End, false),
                (Side::Top, false),
                (Side::Bottom, true),
            ]),
        }
    }
}

impl Layout {
    /// The layout as the file holds it: one line per open panel
    /// (`view centre 0 0 0`: side, column, frame, tab), one per closed one
    /// (`closed files`) and one per edge (`show end false`).
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
        text
    }

    /// Reads a layout file. Lines it doesn't understand are left out (a
    /// line without a column, `code end 0 0`, means column 0); a panel the
    /// file doesn't mention at all (one newer than the file) goes where
    /// the default puts it; a file naming no panel gives the default.
    pub fn from_text(text: &str) -> Layout {
        let mut places = BTreeMap::new();
        let mut closed = Vec::new();
        let mut revealed = Layout::default().revealed;
        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            let (id, side, numbers) = match words.as_slice() {
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
            return Layout::default();
        }
        for (id, place) in Layout::default().places {
            if !closed.contains(&id) {
                places.entry(id).or_insert(place);
            }
        }
        Layout { places, revealed }
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
    <property name="start-width">300</property>
    <property name="end-width">460</property>
    <property name="bottom-height">280</property>
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

/// Where the layout comes from and goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remember {
    /// The user's own file: read at the start, written as it changes.
    Yours,
    /// Board B, and nothing written (screenshots look the same anywhere).
    Nothing,
    /// This file, read once; nothing written.
    From(PathBuf),
}

/// The dock and its panels.
pub struct Workbench {
    /// The dock above the status bar, for the window's content.
    pub root: gtk::Box,
    dock: panel::Dock,
    grid: panel::Grid,
    paneds: BTreeMap<Side, panel::Paned>,
    panels: BTreeMap<PanelId, panel::Widget>,
    /// The status bar's start, for a line about the selection.
    pub status: gtk::Label,
    /// Whether the layout is written to the user's file.
    remembers: bool,
    /// The layout last applied or saved, to bring a closed panel back
    /// where it was.
    last: RefCell<Layout>,
}

impl Workbench {
    /// A dock with each panel's content in its panel, laid out as last
    /// time (or as board B).
    pub fn new(
        contents: Vec<(PanelId, gtk::Widget)>,
        remember: Remember,
    ) -> anyhow::Result<Rc<Workbench>> {
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

        // Frames libpanel makes itself (a panel dropped somewhere empty)
        // get tabs too.
        dock.connect_create_frame(|_, _| new_frame());
        grid.connect_create_frame(|_| new_frame());

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

        let status = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["stave-status-text"])
            .build();
        let statusbar = panel::Statusbar::new();
        statusbar.add_prefix(0, &status);
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
            let toggle = panel::ToggleButton::new(&dock, area);
            statusbar.add_suffix(i32::try_from(priority).unwrap_or(0), &toggle);
        }

        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&dock);
        root.append(&statusbar);

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
        let workbench = Rc::new(Workbench {
            root,
            dock,
            grid,
            paneds,
            panels,
            status,
            remembers,
            last: RefCell::new(layout.clone()),
        });
        workbench.apply(&layout);
        // A panel dropped somewhere new is remembered at once, so closing
        // it later brings it back there.
        let weak = Rc::downgrade(&workbench);
        workbench.dock.connect_panel_drag_end(move |_, _| {
            if let Some(workbench) = weak.upgrade() {
                workbench.save();
            }
        });
        Ok(workbench)
    }

    /// Puts every panel where `layout` says, closing the ones it leaves
    /// out, and shows the edges it shows.
    pub fn apply(&self, layout: &Layout) {
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
            }
        }
        for ((column, row), tabs) in layout.frames(Side::Centre) {
            let frame = self.grid.column(column).row(row);
            self.fill(&frame, &tabs);
        }
        for side in Side::EDGES {
            let shown = layout.revealed.get(&side).copied().unwrap_or(false)
                && !layout.frames(side).is_empty();
            self.dock.set_reveal_area(side.area(), shown);
        }
        self.last.replace(layout.clone());
    }

    /// Adds these panels to a frame, in order.
    fn fill(&self, frame: &panel::Frame, tabs: &[PanelId]) {
        for id in tabs {
            if let Some(widget) = self.panels.get(id) {
                frame.add(widget);
            }
        }
    }

    /// Where the panels are now.
    pub fn layout(&self) -> Layout {
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
        Layout { places, revealed }
    }

    /// Remembers the layout for next time (not while taking screenshots).
    pub fn save(&self) {
        let layout = self.layout();
        self.last.replace(layout.clone());
        if !self.remembers {
            return;
        }
        let path = layout_path();
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(path, layout.to_text());
    }

    /// Board B again.
    pub fn reset(&self) {
        self.apply(&Layout::default());
        self.save();
    }

    /// Shows a panel: brings it back where it last was if it was closed,
    /// shows its edge and makes it the frame's current tab.
    pub fn present(&self, id: PanelId) {
        let Some(widget) = self.panels.get(&id) else {
            return;
        };
        if widget.parent().is_none() {
            let place = self
                .last
                .borrow()
                .places
                .get(&id)
                .copied()
                .or_else(|| Layout::default().places.get(&id).copied());
            if let Some(place) = place {
                let frame = self.frame_at(place);
                self.fill(&frame, &[id]);
            }
        }
        if let Some(side) = widget.position().and_then(|p| Side::of_area(p.area()))
            && side != Side::Centre
        {
            self.dock.set_reveal_area(side.area(), true);
        }
        widget.raise();
    }

    /// Brings a panel to the front of its frame, unless that frame also
    /// holds the 3D View (it would hide the model); a closed panel opens
    /// where it last was. Its edge isn't shown if people hid it.
    pub fn bring_forward_beside_view(&self, id: PanelId) {
        let Some(widget) = self.panels.get(&id) else {
            return;
        };
        let Some(frame) = frame_of(widget) else {
            self.present(id);
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
    pub fn move_to(&self, id: PanelId, side: Side) {
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
        self.save();
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
    frame.set_orientation(gtk::Orientation::Vertical);
    // A slim strip of flush tabs, as in an IDE.
    frame.set_header(Some(&super::tab_strip::TabStrip::new()));
    frame
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_board_b() {
        let layout = Layout::default();
        assert_eq!(
            layout.frames(Side::Start).into_values().collect::<Vec<_>>(),
            vec![vec![PanelId::Parts, PanelId::Files], vec![PanelId::Details]]
        );
        assert_eq!(
            layout.frames(Side::Centre),
            BTreeMap::from([((0, 0), vec![PanelId::View])])
        );
        assert_eq!(
            layout
                .frames(Side::Bottom)
                .into_values()
                .collect::<Vec<_>>(),
            vec![vec![PanelId::Code, PanelId::Problems]]
        );
        assert!(layout.frames(Side::End).is_empty());
        assert_eq!(layout.revealed.get(&Side::End), Some(&false));
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
    fn the_centre_is_never_hidden() {
        let layout = Layout::from_text("view centre 0 0 0\nshow centre false\n");
        assert!(!layout.revealed.contains_key(&Side::Centre));
    }
}
