//! The docked window (docs/superpowers/specs/2026-11-18-docked-window-design.md):
//! a libpanel dock with the 3D view in the centre and the panels round it
//! (Parts, Files, Details, Code and Problems), each of which can be
//! dragged to another edge or into another panel's tabs, closed and
//! brought back. Nothing floats over the 3D view.
//!
//! The default layout is the canvas's board B: Parts (with Files) above
//! Details on the left, Code and Problems along the bottom, the right edge
//! empty and hidden. Where the panels are is remembered per user in
//! `~/.config/stave/layout` (never in a `.stave` file); [`Layout`] is that
//! record, read leniently so an old or broken file only means the default.
//!
//! The Code panel holds the open files as its own tabs, so a file always
//! opens wherever the Code panel is.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use panel::prelude::*;

/// One of the window's panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PanelId {
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
    /// Every panel, in the order the default layout adds them.
    pub const ALL: [PanelId; 5] = [
        PanelId::Parts,
        PanelId::Files,
        PanelId::Details,
        PanelId::Code,
        PanelId::Problems,
    ];

    /// The word the layout file and the actions use.
    pub fn word(self) -> &'static str {
        match self {
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
            PanelId::Parts => "view-list-symbolic",
            PanelId::Files => "folder-symbolic",
            PanelId::Details => "dialog-information-symbolic",
            PanelId::Code => "text-x-generic-symbolic",
            PanelId::Problems => "dialog-warning-symbolic",
        }
    }
}

/// An edge of the window a panel can be docked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Edge {
    /// The left (in left-to-right languages).
    Start,
    /// The right.
    End,
    /// Along the top, under the header.
    Top,
    /// Along the bottom.
    Bottom,
}

impl Edge {
    /// Every edge.
    pub const ALL: [Edge; 4] = [Edge::Start, Edge::End, Edge::Top, Edge::Bottom];

    fn word(self) -> &'static str {
        match self {
            Edge::Start => "start",
            Edge::End => "end",
            Edge::Top => "top",
            Edge::Bottom => "bottom",
        }
    }

    fn from_word(word: &str) -> Option<Edge> {
        Edge::ALL.into_iter().find(|edge| edge.word() == word)
    }

    fn area(self) -> panel::Area {
        match self {
            Edge::Start => panel::Area::Start,
            Edge::End => panel::Area::End,
            Edge::Top => panel::Area::Top,
            Edge::Bottom => panel::Area::Bottom,
        }
    }

    fn of_area(area: panel::Area) -> Option<Edge> {
        match area {
            panel::Area::Start => Some(Edge::Start),
            panel::Area::End => Some(Edge::End),
            panel::Area::Top => Some(Edge::Top),
            panel::Area::Bottom => Some(Edge::Bottom),
            _ => None,
        }
    }
}

/// Where one panel is: an edge, which frame along it (from 0) and which
/// tab in that frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    /// The edge.
    pub edge: Edge,
    /// The frame along the edge, from 0.
    pub frame: u32,
    /// The tab in the frame, from 0.
    pub tab: u32,
}

/// Where the panels are and which edges show: what is remembered between
/// runs. A panel with no place is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Each open panel's place.
    pub places: BTreeMap<PanelId, Place>,
    /// The edges that show.
    pub revealed: BTreeMap<Edge, bool>,
}

impl Default for Layout {
    /// Board B: Parts and Files above Details on the left, Code and
    /// Problems at the bottom, nothing on the right.
    fn default() -> Layout {
        let place = |edge, frame, tab| Place { edge, frame, tab };
        Layout {
            places: BTreeMap::from([
                (PanelId::Parts, place(Edge::Start, 0, 0)),
                (PanelId::Files, place(Edge::Start, 0, 1)),
                (PanelId::Details, place(Edge::Start, 1, 0)),
                (PanelId::Code, place(Edge::Bottom, 0, 0)),
                (PanelId::Problems, place(Edge::Bottom, 0, 1)),
            ]),
            revealed: BTreeMap::from([
                (Edge::Start, true),
                (Edge::End, false),
                (Edge::Top, false),
                (Edge::Bottom, true),
            ]),
        }
    }
}

impl Layout {
    /// The layout as the file holds it: one line per panel
    /// (`parts start 0 0`) and one per edge (`show end false`).
    pub fn to_text(&self) -> String {
        let mut text =
            String::from("# Where Stave's panels are. Delete this file to start again.\n");
        for (id, place) in &self.places {
            let _ = writeln!(
                text,
                "{} {} {} {}",
                id.word(),
                place.edge.word(),
                place.frame,
                place.tab
            );
        }
        for (edge, shown) in &self.revealed {
            let _ = writeln!(text, "show {} {shown}", edge.word());
        }
        text
    }

    /// Reads a layout file. Lines it doesn't understand are left out; a
    /// file naming no panel at all gives the default.
    pub fn from_text(text: &str) -> Layout {
        let mut places = BTreeMap::new();
        let mut revealed = Layout::default().revealed;
        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                ["show", edge, shown] => {
                    if let (Some(edge), Ok(shown)) = (Edge::from_word(edge), shown.parse()) {
                        revealed.insert(edge, shown);
                    }
                }
                [id, edge, frame, tab] => {
                    if let (Some(id), Some(edge), Ok(frame), Ok(tab)) = (
                        PanelId::from_word(id),
                        Edge::from_word(edge),
                        frame.parse(),
                        tab.parse(),
                    ) {
                        places.insert(id, Place { edge, frame, tab });
                    }
                }
                _ => {}
            }
        }
        if places.is_empty() {
            return Layout::default();
        }
        Layout { places, revealed }
    }

    /// The panels at an edge, frame by frame, each frame's in tab order.
    fn frames(&self, edge: Edge) -> Vec<Vec<PanelId>> {
        let mut at: BTreeMap<u32, Vec<(u32, PanelId)>> = BTreeMap::new();
        for (id, place) in &self.places {
            if place.edge == edge {
                at.entry(place.frame).or_default().push((place.tab, *id));
            }
        }
        at.into_values()
            .map(|mut tabs| {
                tabs.sort();
                tabs.into_iter().map(|(_, id)| id).collect()
            })
            .collect()
    }
}

/// The dock's skeleton: an empty pane per edge and a box for the centre.
const DOCK_UI: &str = r#"<interface>
  <object class="PanelDock" id="dock">
    <property name="vexpand">true</property>
    <property name="hexpand">true</property>
    <property name="start-width">300</property>
    <property name="end-width">460</property>
    <property name="bottom-height">280</property>
    <child>
      <object class="GtkBox" id="centre">
        <property name="orientation">vertical</property>
      </object>
    </child>
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
    paneds: BTreeMap<Edge, panel::Paned>,
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
    /// A dock with `centre` in the middle and each panel's content in its
    /// panel, laid out as last time (or as board B).
    pub fn new(
        centre: &impl IsA<gtk::Widget>,
        contents: Vec<(PanelId, gtk::Widget)>,
        remember: Remember,
    ) -> anyhow::Result<Rc<Workbench>> {
        panel::init();
        // libpanel takes the dock's children the way a UI file gives them:
        // a pane of frames per edge, and the centre.
        let builder = gtk::Builder::new();
        builder
            .add_from_string(DOCK_UI)
            .map_err(|error| anyhow::anyhow!("can't lay out the window's panels: {error}"))?;
        let (Some(dock), Some(centre_box)) = (
            builder.object::<panel::Dock>("dock"),
            builder.object::<gtk::Box>("centre"),
        ) else {
            anyhow::bail!("can't lay out the window's panels: the dock is missing");
        };
        centre_box.append(centre);
        let paneds: BTreeMap<Edge, panel::Paned> = Edge::ALL
            .into_iter()
            .filter_map(|edge| Some((edge, builder.object::<panel::Paned>(edge.word())?)))
            .collect();

        // Frames libpanel makes itself (a panel dropped on an empty edge)
        // get tabs too.
        dock.connect_create_frame(|_, _| new_frame());

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
        for (edge, paned) in &self.paneds {
            while let Some(child) = paned.nth_child(0) {
                paned.remove(&child);
            }
            for tabs in layout.frames(*edge) {
                let frame = new_frame();
                for id in tabs {
                    if let Some(widget) = self.panels.get(&id) {
                        frame.add(widget);
                    }
                }
                paned.append(&frame);
            }
        }
        for edge in Edge::ALL {
            let shown = layout.revealed.get(&edge).copied().unwrap_or(false)
                && !layout.frames(edge).is_empty();
            self.dock.set_reveal_area(edge.area(), shown);
        }
        self.last.replace(layout.clone());
    }

    /// Where the panels are now.
    pub fn layout(&self) -> Layout {
        let mut places = BTreeMap::new();
        let mut frame_counts: BTreeMap<Edge, u32> = BTreeMap::new();
        self.dock.foreach_frame(|frame| {
            let Some(edge) = Edge::of_area(frame.position().area()) else {
                return;
            };
            if frame.n_pages() == 0 {
                return;
            }
            let count = frame_counts.entry(edge).or_insert(0);
            let index = *count;
            *count = count.saturating_add(1);
            for tab in 0..frame.n_pages() {
                let id = frame
                    .page(tab)
                    .and_then(|page| PanelId::from_word(&page.id()));
                if let Some(id) = id {
                    places.insert(
                        id,
                        Place {
                            edge,
                            frame: index,
                            tab,
                        },
                    );
                }
            }
        });
        let revealed = Edge::ALL
            .into_iter()
            .map(|edge| (edge, self.dock.reveals_area(edge.area())))
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
                self.frame_at(place.edge, place.frame).add(widget);
            }
        }
        if let Some(edge) = widget.position().and_then(|p| Edge::of_area(p.area())) {
            self.dock.set_reveal_area(edge.area(), true);
        }
        widget.raise();
    }

    /// The `index`th frame along an edge, made (with any before it) if
    /// there aren't that many.
    fn frame_at(&self, edge: Edge, index: u32) -> panel::Frame {
        let mut last = None;
        let mut seen = 0;
        self.dock.foreach_frame(|frame| {
            if Edge::of_area(frame.position().area()) == Some(edge) {
                if seen <= index {
                    last = Some(frame.clone());
                }
                seen += 1;
            }
        });
        match last {
            Some(frame) if seen > index => frame,
            _ => {
                let frame = new_frame();
                if let Some(paned) = self.paneds.get(&edge) {
                    paned.append(&frame);
                }
                frame
            }
        }
    }
}

/// An empty frame, its tabs along the top whichever edge it is at.
fn new_frame() -> panel::Frame {
    let frame = panel::Frame::new();
    frame.set_orientation(gtk::Orientation::Vertical);
    // Tabs with their titles, not libpanel's icon-only switcher.
    frame.set_header(Some(&panel::FrameTabBar::new()));
    frame
}

/// Takes a panel out of whatever frame holds it.
fn detach(widget: &panel::Widget) {
    let mut ancestor = widget.parent();
    while let Some(current) = ancestor {
        if let Some(frame) = current.downcast_ref::<panel::Frame>() {
            frame.remove(widget);
            return;
        }
        ancestor = current.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_board_b() {
        let layout = Layout::default();
        assert_eq!(
            layout.frames(Edge::Start),
            vec![vec![PanelId::Parts, PanelId::Files], vec![PanelId::Details]]
        );
        assert_eq!(
            layout.frames(Edge::Bottom),
            vec![vec![PanelId::Code, PanelId::Problems]]
        );
        assert!(layout.frames(Edge::End).is_empty());
        assert_eq!(layout.revealed.get(&Edge::End), Some(&false));
    }

    #[test]
    fn a_layout_reads_back_as_written() {
        let mut layout = Layout::default();
        layout.places.insert(
            PanelId::Code,
            Place {
                edge: Edge::End,
                frame: 0,
                tab: 0,
            },
        );
        layout.revealed.insert(Edge::End, true);
        assert_eq!(Layout::from_text(&layout.to_text()), layout);
    }

    #[test]
    fn a_broken_file_gives_the_default() {
        assert_eq!(Layout::from_text(""), Layout::default());
        assert_eq!(Layout::from_text("nonsense\n\u{0}\u{1}"), Layout::default());
    }

    #[test]
    fn lines_naming_unknown_panels_are_left_out() {
        let layout = Layout::from_text("parts start 0 0\nruler end 0 0\ncode end x 0\n");
        assert_eq!(layout.places.len(), 1);
        assert!(layout.places.contains_key(&PanelId::Parts));
    }
}
