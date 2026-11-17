//! The code editor of the Files view (FilesMode.dc.html): a text view
//! with line numbers, `.stave` colours from the navigation layer's
//! semantic tokens, wavy underlines for problems, the uses of the name
//! under the cursor marked, lenses above declarations ("2 uses · show in
//! 3D") and an inline peek of a definition under a line.
//!
//! It only displays: what to colour, underline, mark and show comes from
//! the Files page (`files.rs`), which asks `stave-ide`. Positions arrive
//! as byte offsets and are turned into lines and byte indexes here
//! ([`LineStarts`]), the only place that knows GTK counts in characters.
//!
//! GtkSourceView 5 isn't installed on the build machines (only the GTK 3
//! version 4 is), so the editor is a plain `GtkTextView`: lenses and the
//! peek are overlay children placed in the room a tag's
//! `pixels-above-lines` / `pixels-below-lines` makes for them, and the line
//! numbers are a gutter widget of our own.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;
use stave_ide::{SymbolId, SymbolKind, TokenClass};
use stave_lang::Severity;

/// The text's own spacing above and below each line: 13 px type on
/// 23 px lines, as the design has it.
const LINE_SPACING: i32 = 3;

/// The room a lens takes above its line.
const LENS_HEIGHT: i32 = 20;

/// The gutter's width.
const GUTTER_WIDTH: i32 = 52;

/// Where each line of a text starts, in bytes, to turn the navigation
/// layer's byte offsets into GTK's lines and byte indexes and back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineStarts(Vec<u32>);

impl LineStarts {
    /// The line starts of `text`.
    pub fn new(text: &str) -> LineStarts {
        let mut starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n'
                && let Ok(next) = u32::try_from(index.saturating_add(1))
            {
                starts.push(next);
            }
        }
        LineStarts(starts)
    }

    /// The 0-based line and the byte index in it of a byte offset.
    pub fn position(&self, offset: u32) -> (i32, i32) {
        let line = self
            .0
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let start = self.0.get(line).copied().unwrap_or(0);
        (
            i32::try_from(line).unwrap_or(i32::MAX),
            i32::try_from(offset.saturating_sub(start)).unwrap_or(i32::MAX),
        )
    }

    /// The byte offset of a 0-based line and a byte index in it.
    pub fn offset(&self, line: i32, index: i32) -> u32 {
        let start = usize::try_from(line)
            .ok()
            .and_then(|line| self.0.get(line))
            .copied()
            .unwrap_or(0);
        start.saturating_add(u32::try_from(index).unwrap_or(0))
    }
}

/// The iterator at a byte offset of a buffer whose text has these line
/// starts.
pub fn iter_at(buffer: &gtk::TextBuffer, lines: &LineStarts, offset: u32) -> gtk::TextIter {
    let (line, index) = lines.position(offset);
    buffer
        .iter_at_line_index(line, index)
        .or_else(|| buffer.iter_at_line(line))
        .unwrap_or_else(|| buffer.end_iter())
}

/// The byte offset of an iterator.
pub fn offset_of(lines: &LineStarts, iter: &gtk::TextIter) -> u32 {
    lines.offset(iter.line(), iter.line_index())
}

/// The tags every buffer shares.
struct Tags {
    keyword: gtk::TextTag,
    comment: gtk::TextTag,
    doc: gtk::TextTag,
    text: gtk::TextTag,
    number: gtk::TextTag,
    function: gtk::TextTag,
    kind: gtk::TextTag,
    module: gtk::TextTag,
    word: gtk::TextTag,
    error: gtk::TextTag,
    warning: gtk::TextTag,
    declared: gtk::TextTag,
    used: gtk::TextTag,
    cursor_line: gtk::TextTag,
    lens: gtk::TextTag,
    peek: gtk::TextTag,
}

impl Tags {
    fn new(table: &gtk::TextTagTable) -> Tags {
        let tag = |name: &str| {
            let tag = gtk::TextTag::new(Some(name));
            table.add(&tag);
            tag
        };
        let tags = Tags {
            cursor_line: tag("cursor-line"),
            used: tag("used"),
            declared: tag("declared"),
            keyword: tag("keyword"),
            comment: tag("comment"),
            doc: tag("doc"),
            text: tag("text"),
            number: tag("number"),
            function: tag("function"),
            kind: tag("type"),
            module: tag("module"),
            word: tag("word"),
            error: tag("error"),
            warning: tag("warning"),
            lens: tag("lens"),
            peek: tag("peek"),
        };
        tags.comment.set_style(gtk::pango::Style::Italic);
        tags.doc.set_style(gtk::pango::Style::Italic);
        tags.kind.set_weight(600);
        tags.declared.set_weight(600);
        tags.lens
            .set_pixels_above_lines(LINE_SPACING.saturating_add(LENS_HEIGHT));
        tags
    }

    fn class(&self, class: TokenClass) -> Option<&gtk::TextTag> {
        match class {
            TokenClass::Keyword => Some(&self.keyword),
            TokenClass::Comment => Some(&self.comment),
            TokenClass::DocComment => Some(&self.doc),
            TokenClass::Text => Some(&self.text),
            TokenClass::Number => Some(&self.number),
            TokenClass::Function => Some(&self.function),
            TokenClass::Type => Some(&self.kind),
            TokenClass::Module => Some(&self.module),
            TokenClass::EnumWord => Some(&self.word),
            TokenClass::Error => Some(&self.error),
            TokenClass::Item(kind) => match kind {
                SymbolKind::Component | SymbolKind::Kind => Some(&self.kind),
                SymbolKind::Module => Some(&self.module),
                SymbolKind::Recipe
                | SymbolKind::Stock
                | SymbolKind::Profile
                | SymbolKind::Scene
                | SymbolKind::Param
                | SymbolKind::Let
                | SymbolKind::Region
                | SymbolKind::Part
                | SymbolKind::Joint
                | SymbolKind::Hardware
                | SymbolKind::Assembly
                | SymbolKind::Require
                | SymbolKind::Role
                | SymbolKind::MadePart
                | SymbolKind::Member
                | SymbolKind::Label => None,
            },
            TokenClass::Field | TokenClass::Punctuation | TokenClass::Plain => None,
        }
    }

    fn syntax(&self) -> [&gtk::TextTag; 9] {
        [
            &self.keyword,
            &self.comment,
            &self.doc,
            &self.text,
            &self.number,
            &self.function,
            &self.kind,
            &self.module,
            &self.word,
        ]
    }

    /// The design's colours (FilesMode.dc.html), and lighter ones for the
    /// dark style.
    fn colour(&self, dark: bool) {
        let rgba = |hex: &str| gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::BLACK);
        let set = |tag: &gtk::TextTag, light: &str, darker: &str| {
            tag.set_foreground_rgba(Some(&rgba(if dark { darker } else { light })));
        };
        set(&self.keyword, "#9141ac", "#dc8add");
        set(&self.comment, "#6f6e73", "#9a9996");
        set(&self.doc, "#6f6e73", "#9a9996");
        set(&self.text, "#1b7a4c", "#8ff0a4");
        set(&self.number, "#c64600", "#ffa348");
        set(&self.function, "#1c71d8", "#78aeed");
        set(&self.kind, "#1a5fb4", "#99c1f1");
        set(&self.module, "#3d3846", "#deddda");
        set(&self.word, "#1c71d8", "#78aeed");
        self.declared
            .set_background_rgba(Some(&rgba(if dark { "#1e3a5f" } else { "#d3e4f9" })));
        self.used
            .set_background_rgba(Some(&rgba(if dark { "#25344a" } else { "#e6eefa" })));
        self.cursor_line
            .set_paragraph_background_rgba(Some(&rgba(if dark { "#26303d" } else { "#f1f6fd" })));
    }
}

mod imp {
    use super::*;

    /// The line numbers beside the text.
    #[derive(Default)]
    pub struct Gutter {
        pub view: glib::WeakRef<gtk::TextView>,
        /// The cursor's line, from 0, drawn in the accent colour.
        pub current: Cell<i32>,
        pub dark: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Gutter {
        const NAME: &'static str = "StaveGutter";
        type Type = super::Gutter;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Gutter {}

    impl WidgetImpl for Gutter {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (GUTTER_WIDTH, GUTTER_WIDTH, -1, -1),
                _ => (0, 0, -1, -1),
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(view) = self.view.upgrade() else {
                return;
            };
            let widget = self.obj();
            let buffer = view.buffer();
            let visible = view.visible_rect();
            let (mut iter, _) = view.line_at_y(visible.y());
            let bottom = visible.y().saturating_add(visible.height());
            let dim = {
                let colour = widget.color();
                gtk::gdk::RGBA::new(colour.red(), colour.green(), colour.blue(), 0.45)
            };
            let accent = gtk::gdk::RGBA::parse(if self.dark.get() {
                "#78aeed"
            } else {
                "#1c71d8"
            })
            .unwrap_or(dim);
            let width = widget.width();
            for _ in 0..10_000 {
                let location = view.iter_location(&iter);
                if location.y() > bottom {
                    break;
                }
                let (_, y) =
                    view.buffer_to_window_coords(gtk::TextWindowType::Left, 0, location.y());
                let line = iter.line();
                let current = line == self.current.get();
                let layout = widget.create_pango_layout(Some(&line.saturating_add(1).to_string()));
                if current {
                    let attributes = gtk::pango::AttrList::new();
                    attributes.insert(gtk::pango::AttrInt::new_weight(
                        gtk::pango::Weight::Semibold,
                    ));
                    layout.set_attributes(Some(&attributes));
                }
                let (text_width, text_height) = layout.pixel_size();
                let x = width.saturating_sub(14).saturating_sub(text_width);
                let y = y.saturating_add((location.height().saturating_sub(text_height)) / 2);
                snapshot.save();
                snapshot.translate(&gtk::graphene::Point::new(
                    crate::convert::f32_of_int(x),
                    crate::convert::f32_of_int(y),
                ));
                snapshot.append_layout(&layout, if current { &accent } else { &dim });
                snapshot.restore();
                if let Some(severity) = line_problem(&view, &iter) {
                    let colour = problem_colour(severity, self.dark.get());
                    let bar = gtk::graphene::Rect::new(
                        4.0,
                        crate::convert::f32_of_int(y),
                        3.0,
                        crate::convert::f32_of_int(text_height),
                    );
                    snapshot.append_color(&colour, &bar);
                }
                if !iter.forward_line() {
                    break;
                }
            }
            let _ = buffer;
        }
    }

    /// The text view: a `GtkTextView` that draws problems' wavy
    /// underlines itself, thicker than Pango's, in libadwaita's error and
    /// warning colours.
    #[derive(Default)]
    pub struct CodeView {
        pub dark: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CodeView {
        const NAME: &'static str = "StaveCodeView";
        type Type = super::CodeView;
        type ParentType = gtk::TextView;
    }

    impl ObjectImpl for CodeView {}
    impl WidgetImpl for CodeView {}

    impl TextViewImpl for CodeView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            self.parent_snapshot_layer(layer, snapshot.clone());
            if layer != gtk::TextViewLayer::AboveText {
                return;
            }
            let view = self.obj();
            let view: &gtk::TextView = view.upcast_ref();
            let buffer = view.buffer();
            let table = buffer.tag_table();
            let dark = self.dark.get();
            for (name, severity) in [("warning", Severity::Warning), ("error", Severity::Error)] {
                let Some(tag) = table.lookup(name) else {
                    continue;
                };
                let colour = problem_colour(severity, dark);
                draw_waves(view, &tag, &colour, &snapshot);
            }
        }
    }
}

glib::wrapper! {
    /// The line numbers beside the editor's text.
    pub struct Gutter(ObjectSubclass<imp::Gutter>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

glib::wrapper! {
    /// The editor's text view.
    pub struct CodeView(ObjectSubclass<imp::CodeView>)
        @extends gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

/// The worst problem underlined on the line that starts at `line`.
fn line_problem(view: &gtk::TextView, line: &gtk::TextIter) -> Option<Severity> {
    let table = view.buffer().tag_table();
    let mut end = *line;
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    [("error", Severity::Error), ("warning", Severity::Warning)]
        .into_iter()
        .find(|(name, _)| {
            table.lookup(name).is_some_and(|tag| {
                let mut at = *line;
                at.has_tag(&tag)
                    || (at.forward_to_tag_toggle(Some(&tag)) && at.offset() < end.offset())
            })
        })
        .map(|(_, severity)| severity)
}

/// libadwaita's error and warning colours (`@error_color`,
/// `@warning_color`) for the light or dark style.
fn problem_colour(severity: Severity, dark: bool) -> gtk::gdk::RGBA {
    let hex = match (severity, dark) {
        (Severity::Error, false) => "#c01c28",
        (Severity::Error, true) => "#ff7b63",
        (Severity::Warning | Severity::Note, false) => "#9c6e03",
        (Severity::Warning | Severity::Note, true) => "#f8e45c",
    };
    gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::RED)
}

/// Draws a wave under every visible run of `tag`, line by line, in
/// buffer coordinates (the layer's).
fn draw_waves(
    view: &gtk::TextView,
    tag: &gtk::TextTag,
    colour: &gtk::gdk::RGBA,
    snapshot: &gtk::Snapshot,
) {
    let visible = view.visible_rect();
    let (mut iter, _) = view.line_at_y(visible.y());
    let (mut last, _) = view.line_at_y(visible.y().saturating_add(visible.height()));
    last.forward_to_line_end();
    let end = last.offset();
    for _ in 0..10_000 {
        if !iter.has_tag(tag) && !iter.forward_to_tag_toggle(Some(tag)) {
            return;
        }
        if iter.offset() >= end || !iter.has_tag(tag) {
            return;
        }
        let mut run_end = iter;
        run_end.forward_to_tag_toggle(Some(tag));
        // One wave per line the run covers.
        let mut from = iter;
        for _ in 0..10_000 {
            let mut line_end = from;
            if !line_end.ends_line() {
                line_end.forward_to_line_end();
            }
            let to = if line_end.offset() < run_end.offset() {
                line_end
            } else {
                run_end
            };
            let start = view.iter_location(&from);
            let stop = view.iter_location(&to);
            if stop.x() > start.x() {
                wave(
                    snapshot,
                    f64::from(start.x()),
                    f64::from(stop.x()),
                    f64::from(start.y().saturating_add(start.height())) + 1.0,
                    colour,
                );
            }
            if to.offset() >= run_end.offset() || !from.forward_line() {
                break;
            }
        }
        iter = run_end;
    }
}

/// A wavy line from `x0` to `x1` around `y`: 1.6 px thick, 1.6 px either
/// side of the line, a crest every 5 px.
fn wave(snapshot: &gtk::Snapshot, x0: f64, x1: f64, y: f64, colour: &gtk::gdk::RGBA) {
    let width = (x1 - x0).max(4.0);
    let bounds = gtk::graphene::Rect::new(
        crate::convert::f32_of(x0 - 1.0),
        crate::convert::f32_of(y - 3.0),
        crate::convert::f32_of(width + 2.0),
        6.0,
    );
    let context = snapshot.append_cairo(&bounds);
    context.set_source_rgba(
        f64::from(colour.red()),
        f64::from(colour.green()),
        f64::from(colour.blue()),
        f64::from(colour.alpha()),
    );
    context.set_line_width(1.6);
    context.set_line_join(gtk::cairo::LineJoin::Round);
    context.move_to(x0, y);
    let (step, amplitude) = (2.5, 1.6);
    let mut x = x0;
    let mut up = true;
    while x < x0 + width {
        x = (x + step).min(x0 + width);
        context.line_to(x, if up { y - amplitude } else { y + amplitude });
        up = !up;
    }
    // A failed stroke only leaves the wave out.
    let _ = context.stroke();
}

impl Gutter {
    fn new(view: &gtk::TextView) -> Gutter {
        let gutter: Gutter = glib::Object::builder().build();
        gutter.imp().view.set(Some(view));
        gutter.add_css_class("stave-gutter");
        gutter
    }
}

/// A coloured run, as lines and byte indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// Its first byte.
    pub start: u32,
    /// One past its last byte.
    pub end: u32,
}

/// The editor widget.
pub struct Editor {
    /// The editor, ready to pack: the text in a scrolled window.
    pub root: gtk::ScrolledWindow,
    /// The text view.
    pub view: gtk::TextView,
    table: gtk::TextTagTable,
    tags: Tags,
    gutter: Gutter,
    /// The lens rows, made as needed and kept (GTK can't take an overlay
    /// back out of a text view, so they are hidden when not used).
    lenses: RefCell<Vec<LensSlot>>,
    /// How many of the lens rows are in use.
    lenses_used: Cell<usize>,
    /// Where the peek goes, and the line it sits under while it shows.
    peek_box: gtk::Box,
    peek_line: Cell<Option<i32>>,
    /// Where the hover card goes.
    card_box: gtk::Box,
    on_uses: LensCallback,
    on_show: LensCallback,
}

/// What a lens's link calls, set once the page connects it.
type LensCallback = Rc<RefCell<Option<Rc<dyn Fn(SymbolId)>>>>;

/// What a lens says: "2 uses · show in 3D" above a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LensLine {
    /// The line it sits above, from 0.
    pub line: i32,
    /// "2 uses".
    pub uses: String,
    /// Whether it offers "show in 3D".
    pub shows_in_3d: bool,
    /// The declaration.
    pub symbol: SymbolId,
    /// The declaration's name, for the tooltips.
    pub name: String,
}

/// One lens row.
struct LensSlot {
    root: gtk::Box,
    uses: gtk::Button,
    dot: gtk::Label,
    show: gtk::Button,
    symbol: Rc<Cell<Option<SymbolId>>>,
    line: Cell<i32>,
}

impl Editor {
    /// An empty editor.
    pub fn new() -> Editor {
        let table = gtk::TextTagTable::new();
        let tags = Tags::new(&table);
        let view: gtk::TextView = glib::Object::builder::<CodeView>()
            .property("monospace", true)
            .property("wrap-mode", gtk::WrapMode::None)
            .property("top-margin", 12)
            .property("bottom-margin", 48)
            .property("left-margin", 2)
            .property("right-margin", 16)
            .property("pixels-above-lines", LINE_SPACING)
            .property("pixels-below-lines", LINE_SPACING)
            .property("css-classes", ["stave-editor"].to_value())
            .build()
            .upcast();
        view.set_buffer(Some(&gtk::TextBuffer::new(Some(&table))));
        let gutter = Gutter::new(&view);
        view.set_gutter(gtk::TextWindowType::Left, Some(&gutter));
        let root = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hexpand(true)
            .build();
        let redraw = gutter.downgrade();
        root.vadjustment().connect_value_changed(move |_| {
            if let Some(gutter) = redraw.upgrade() {
                gutter.queue_draw();
            }
        });
        // A wider editor scrolls back to the start of the lines when the
        // cursor fits without scrolling.
        let text = view.downgrade();
        root.hadjustment().connect_changed(move |horizontal| {
            let Some(view) = text.upgrade() else { return };
            let buffer = view.buffer();
            let cursor = view.iter_location(&buffer.iter_at_mark(&buffer.get_insert()));
            if f64::from(cursor.x() + 24) <= horizontal.page_size() && horizontal.value() > 0.0 {
                horizontal.set_value(0.0);
            }
        });
        let peek_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .visible(false)
            .build();
        view.add_overlay(&peek_box, 0, 0);
        let card_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .visible(false)
            .build();
        view.add_overlay(&card_box, 0, 0);
        let editor = Editor {
            root,
            view,
            table,
            tags,
            gutter,
            lenses: RefCell::new(Vec::new()),
            lenses_used: Cell::new(0),
            peek_box,
            peek_line: Cell::new(None),
            card_box,
            on_uses: Rc::new(RefCell::new(None)),
            on_show: Rc::new(RefCell::new(None)),
        };
        editor.colour(adw::StyleManager::default().is_dark());
        editor
    }

    /// A new buffer for a document, sharing the editor's tags.
    pub fn new_buffer(&self, text: &str) -> gtk::TextBuffer {
        let buffer = gtk::TextBuffer::new(Some(&self.table));
        buffer.set_text(text);
        buffer.place_cursor(&buffer.start_iter());
        buffer.set_enable_undo(true);
        let redraw = self.gutter.downgrade();
        buffer.connect_changed(move |_| {
            if let Some(gutter) = redraw.upgrade() {
                gutter.queue_draw();
            }
        });
        buffer
    }

    /// Shows a buffer; read-only documents can't be typed in.
    pub fn show_buffer(&self, buffer: &gtk::TextBuffer, editable: bool) {
        self.clear_overlays();
        self.view.set_buffer(Some(buffer));
        self.view.set_editable(editable);
        self.view.set_cursor_visible(true);
        self.gutter.queue_draw();
    }

    /// Follows the light or dark style.
    pub fn colour(&self, dark: bool) {
        self.tags.colour(dark);
        if let Some(code) = self.view.downcast_ref::<CodeView>() {
            code.imp().dark.set(dark);
        }
        self.view.queue_draw();
        self.gutter.imp().dark.set(dark);
        self.gutter.queue_draw();
    }

    /// Colours a buffer by its tokens.
    pub fn paint(&self, buffer: &gtk::TextBuffer, lines: &LineStarts, runs: &[(Run, TokenClass)]) {
        let (start, end) = buffer.bounds();
        for tag in self.tags.syntax() {
            buffer.remove_tag(tag, &start, &end);
        }
        for (run, class) in runs {
            if let Some(tag) = self.tags.class(*class) {
                buffer.apply_tag(
                    tag,
                    &iter_at(buffer, lines, run.start),
                    &iter_at(buffer, lines, run.end),
                );
            }
        }
    }

    /// Underlines problems: errors red, warnings amber.
    pub fn underline(
        &self,
        buffer: &gtk::TextBuffer,
        lines: &LineStarts,
        problems: &[(Run, Severity)],
    ) {
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.error, &start, &end);
        buffer.remove_tag(&self.tags.warning, &start, &end);
        for (run, severity) in problems {
            let tag = match severity {
                Severity::Error => &self.tags.error,
                Severity::Warning | Severity::Note => &self.tags.warning,
            };
            let from = iter_at(buffer, lines, run.start);
            let mut to = iter_at(buffer, lines, run.end);
            // An empty span still shows: underline the character after it,
            // or the one before at the end of a line.
            if to.offset() <= from.offset() {
                to = from;
                if to.ends_line() {
                    let mut before = from;
                    before.backward_char();
                    buffer.apply_tag(tag, &before, &from);
                    continue;
                }
                to.forward_char();
            }
            buffer.apply_tag(tag, &from, &to);
        }
        self.gutter.queue_draw();
        self.view.queue_draw();
    }

    /// Marks the uses of the name under the cursor, its declaration
    /// stronger.
    pub fn mark_uses(
        &self,
        buffer: &gtk::TextBuffer,
        lines: &LineStarts,
        declared: Option<Run>,
        uses: &[Run],
    ) {
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.declared, &start, &end);
        buffer.remove_tag(&self.tags.used, &start, &end);
        for run in uses {
            buffer.apply_tag(
                &self.tags.used,
                &iter_at(buffer, lines, run.start),
                &iter_at(buffer, lines, run.end),
            );
        }
        if let Some(run) = declared {
            buffer.apply_tag(
                &self.tags.declared,
                &iter_at(buffer, lines, run.start),
                &iter_at(buffer, lines, run.end),
            );
        }
    }

    /// Tints the cursor's line and its number.
    pub fn mark_cursor_line(&self, buffer: &gtk::TextBuffer) {
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.cursor_line, &start, &end);
        let cursor = buffer.iter_at_mark(&buffer.get_insert());
        let line = cursor.line();
        if let Some(from) = buffer.iter_at_line(line) {
            let mut to = from;
            to.forward_line();
            buffer.apply_tag(&self.tags.cursor_line, &from, &to);
        }
        self.gutter.imp().current.set(line);
        self.gutter.queue_draw();
    }

    fn clear_overlays(&self) {
        self.set_lenses(&[]);
        self.set_peek(None);
        self.hide_card();
    }

    /// A lens's "2 uses" was clicked.
    pub fn connect_lens_uses(&self, callback: impl Fn(SymbolId) + 'static) {
        *self.on_uses.borrow_mut() = Some(Rc::new(callback));
    }

    /// A lens's "show in 3D" was clicked.
    pub fn connect_lens_show(&self, callback: impl Fn(SymbolId) + 'static) {
        *self.on_show.borrow_mut() = Some(Rc::new(callback));
    }

    fn lens_slot(&self) -> LensSlot {
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .css_classes(["stave-lens"])
            .build();
        let uses = gtk::Button::builder()
            .css_classes(["flat", "stave-lens-link"])
            .build();
        let dot = gtk::Label::new(Some("·"));
        dot.add_css_class("stave-lens-dot");
        let show = gtk::Button::builder()
            .label("show in 3D")
            .css_classes(["flat", "stave-lens-link"])
            .build();
        root.append(&uses);
        root.append(&dot);
        root.append(&show);
        let symbol: Rc<Cell<Option<SymbolId>>> = Rc::new(Cell::new(None));
        for (button, callbacks) in [(&uses, &self.on_uses), (&show, &self.on_show)] {
            let symbol = symbol.clone();
            let callbacks = callbacks.clone();
            button.connect_clicked(move |_| {
                let callback = callbacks.borrow().clone();
                if let (Some(callback), Some(symbol)) = (callback, symbol.get()) {
                    callback(symbol);
                }
            });
        }
        self.view.add_overlay(&root, 0, 0);
        LensSlot {
            root,
            uses,
            dot,
            show,
            symbol,
            line: Cell::new(0),
        }
    }

    /// Puts a lens above each of these lines, replacing the ones there.
    pub fn set_lenses(&self, lenses: &[LensLine]) {
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.lens, &start, &end);
        while self.lenses.borrow().len() < lenses.len() {
            let slot = self.lens_slot();
            self.lenses.borrow_mut().push(slot);
        }
        for (index, slot) in self.lenses.borrow().iter().enumerate() {
            match lenses.get(index) {
                Some(lens) => {
                    slot.uses.set_label(&lens.uses);
                    slot.uses.set_tooltip_text(Some(&format!(
                        "List the uses of {} (Shift+F12)",
                        lens.name
                    )));
                    slot.show
                        .set_tooltip_text(Some(&format!("Select {} in the 3D view", lens.name)));
                    slot.dot.set_visible(lens.shows_in_3d);
                    slot.show.set_visible(lens.shows_in_3d);
                    slot.symbol.set(Some(lens.symbol));
                    slot.line.set(lens.line);
                    slot.root.set_visible(true);
                    if let Some(from) = buffer.iter_at_line(lens.line) {
                        let mut to = from;
                        to.forward_to_line_end();
                        buffer.apply_tag(&self.tags.lens, &from, &to);
                    }
                }
                None => {
                    slot.root.set_visible(false);
                    slot.symbol.set(None);
                }
            }
        }
        self.lenses_used.set(lenses.len());
        self.place_overlays();
    }

    /// Shows a peek under a line (from 0), or none.
    pub fn set_peek(&self, peek: Option<(gtk::Widget, i32)>) {
        while let Some(child) = self.peek_box.first_child() {
            self.peek_box.remove(&child);
        }
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.peek, &start, &end);
        match peek {
            Some((widget, line)) => {
                self.peek_box.append(&widget);
                self.peek_box.set_visible(true);
                self.peek_line.set(Some(line));
            }
            None => {
                self.peek_box.set_visible(false);
                self.peek_line.set(None);
            }
        }
        self.size_peek();
        self.place_overlays();
    }

    /// `true` while a peek is open.
    pub fn has_peek(&self) -> bool {
        self.peek_line.get().is_some()
    }

    /// Shows a card (the hover card) at a point of the text, in buffer
    /// coordinates.
    pub fn show_card(&self, card: &gtk::Widget, x: i32, y: i32) {
        while let Some(child) = self.card_box.first_child() {
            self.card_box.remove(&child);
        }
        self.card_box.append(card);
        self.card_box.set_visible(true);
        self.view.move_overlay(&self.card_box, x, y);
    }

    /// Hides the hover card.
    pub fn hide_card(&self) {
        self.card_box.set_visible(false);
        while let Some(child) = self.card_box.first_child() {
            self.card_box.remove(&child);
        }
    }

    /// `true` while the hover card shows.
    pub fn has_card(&self) -> bool {
        self.card_box.is_visible()
    }

    /// Fits the peek to the editor's width and makes room for it above the
    /// next line, so the cursor's line keeps its own tint.
    fn size_peek(&self) {
        let Some(line) = self.peek_line.get() else {
            return;
        };
        let width = self
            .root
            .width()
            .saturating_sub(GUTTER_WIDTH)
            .saturating_sub(16)
            .max(400);
        if self.peek_box.width_request() != width {
            self.peek_box.set_size_request(width, -1);
        }
        let (_, natural, _, _) = self.peek_box.measure(gtk::Orientation::Vertical, width);
        let next = line.saturating_add(1);
        let used = self.lenses_used.get();
        let lens_below = self
            .lenses
            .borrow()
            .iter()
            .take(used)
            .any(|slot| slot.line.get() == next);
        let mut room = LINE_SPACING.saturating_add(natural).saturating_add(10);
        if lens_below {
            room = room.saturating_add(LENS_HEIGHT);
        }
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.peek, &start, &end);
        match buffer
            .iter_at_line(next)
            .filter(|_| next < buffer.line_count())
        {
            Some(from) => {
                if self.tags.peek.pixels_above_lines() != room {
                    self.tags.peek.set_pixels_above_lines(room);
                }
                self.tags.peek.set_pixels_below_lines(LINE_SPACING);
                let mut to = from;
                to.forward_to_line_end();
                buffer.apply_tag(&self.tags.peek, &from, &to);
            }
            None => {
                self.tags.peek.set_pixels_above_lines(LINE_SPACING);
                self.tags.peek.set_pixels_below_lines(room);
                if let Some(from) = buffer.iter_at_line(line) {
                    let mut to = from;
                    to.forward_to_line_end();
                    buffer.apply_tag(&self.tags.peek, &from, &to);
                }
            }
        }
    }

    /// Moves the lenses and the peek to their lines, after the text moved.
    pub fn place_overlays(&self) {
        self.size_peek();
        let buffer = self.view.buffer();
        let used = self.lenses_used.get();
        for slot in self.lenses.borrow().iter().take(used) {
            if let Some(iter) = buffer.iter_at_line(slot.line.get()) {
                // Just above the line's text, whatever room is above it.
                let text_top = self.view.iter_location(&iter).y();
                let y = text_top
                    .saturating_sub(LINE_SPACING)
                    .saturating_sub(LENS_HEIGHT)
                    .saturating_add(2);
                self.view.move_overlay(&slot.root, 2, y);
            }
        }
        if let Some(line) = self.peek_line.get()
            && let Some(iter) = buffer.iter_at_line(line)
        {
            let location = self.view.iter_location(&iter);
            let y = location
                .y()
                .saturating_add(location.height())
                .saturating_add(LINE_SPACING)
                .saturating_add(4);
            self.view.move_overlay(&self.peek_box, 0, y);
        }
    }

    /// Puts the cursor at a byte offset and scrolls it into view, about a
    /// third of the way down.
    pub fn go_to(&self, lines: &LineStarts, offset: u32) {
        let buffer = self.view.buffer();
        let iter = iter_at(&buffer, lines, offset);
        buffer.place_cursor(&iter);
        let mark = buffer.get_insert();
        // Scroll once the text is laid out: up and down only, so the line
        // keeps its start in view, unless the cursor is off to the right.
        let view = self.view.downgrade();
        let scrolled = self.root.downgrade();
        glib::idle_add_local_once(move || {
            let (Some(view), Some(scrolled)) = (view.upgrade(), scrolled.upgrade()) else {
                return;
            };
            let buffer = view.buffer();
            let cursor = buffer.iter_at_mark(&mark);
            let location = view.iter_location(&cursor);
            let vertical = scrolled.vadjustment();
            let page = vertical.page_size();
            let wanted = f64::from(location.y()) - page * 0.3;
            let top = f64::from(location.y()) >= vertical.value()
                && f64::from(location.y() + location.height()) <= vertical.value() + page;
            if !top || page <= 1.0 {
                vertical
                    .set_value(wanted.clamp(vertical.lower(), (vertical.upper() - page).max(0.0)));
            }
            let horizontal = scrolled.hadjustment();
            let right = f64::from(location.x() + 24);
            let page = horizontal.page_size();
            if page > 50.0 {
                horizontal.set_value(if right > page { right - page } else { 0.0 });
            }
        });
    }

    /// The cursor's byte offset.
    pub fn cursor(&self, lines: &LineStarts) -> u32 {
        let buffer = self.view.buffer();
        offset_of(lines, &buffer.iter_at_mark(&buffer.get_insert()))
    }

    /// The byte offset under a point of the text view, in widget
    /// coordinates.
    pub fn offset_at(&self, lines: &LineStarts, x: f64, y: f64) -> Option<u32> {
        let (bx, by) = self.view.window_to_buffer_coords(
            gtk::TextWindowType::Widget,
            crate::convert::int_of(x),
            crate::convert::int_of(y),
        );
        let iter = self.view.iter_at_location(bx, by)?;
        Some(offset_of(lines, &iter))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_offsets_turn_into_lines_and_back() {
        let lines = LineStarts::new("ab\ncdé\n\nx");
        assert_eq!(lines.position(0), (0, 0));
        assert_eq!(lines.position(4), (1, 1));
        assert_eq!(lines.position(8), (2, 0));
        assert_eq!(lines.position(9), (3, 0));
        assert_eq!(lines.offset(1, 1), 4);
        assert_eq!(lines.offset(3, 0), 9);
        assert_eq!(lines.offset(99, 0), 0);
    }
}
