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

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;
use stave_ide::{SymbolKind, TokenClass};
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
        tags.error.set_underline(gtk::pango::Underline::Error);
        tags.warning.set_underline(gtk::pango::Underline::Error);
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
        self.error
            .set_underline_rgba(Some(&rgba(if dark { "#ff7b63" } else { "#c01c28" })));
        self.warning
            .set_underline_rgba(Some(&rgba(if dark { "#f8e45c" } else { "#c88800" })));
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
                if !iter.forward_line() {
                    break;
                }
            }
            let _ = buffer;
        }
    }
}

glib::wrapper! {
    /// The line numbers beside the editor's text.
    pub struct Gutter(ObjectSubclass<imp::Gutter>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
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
    /// The lens widgets and the lines they sit above.
    lenses: RefCell<Vec<(gtk::Widget, i32)>>,
    /// The peek and the line it sits under.
    peek: RefCell<Option<(gtk::Widget, i32)>>,
}

impl Editor {
    /// An empty editor.
    pub fn new() -> Editor {
        let table = gtk::TextTagTable::new();
        let tags = Tags::new(&table);
        let view = gtk::TextView::builder()
            .monospace(true)
            .wrap_mode(gtk::WrapMode::None)
            .top_margin(12)
            .bottom_margin(48)
            .left_margin(2)
            .right_margin(16)
            .pixels_above_lines(LINE_SPACING)
            .pixels_below_lines(LINE_SPACING)
            .css_classes(["stave-editor"])
            .has_tooltip(true)
            .build();
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
        let editor = Editor {
            root,
            view,
            table,
            tags,
            gutter,
            lenses: RefCell::new(Vec::new()),
            peek: RefCell::new(None),
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
        for (widget, _) in self.lenses.borrow_mut().drain(..) {
            self.view.remove(&widget);
        }
        if let Some((widget, _)) = self.peek.borrow_mut().take() {
            self.view.remove(&widget);
        }
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.lens, &start, &end);
        buffer.remove_tag(&self.tags.peek, &start, &end);
    }

    /// Puts a lens widget above each of these lines (from 0), replacing
    /// the ones there.
    pub fn set_lenses(&self, lenses: Vec<(gtk::Widget, i32)>) {
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.lens, &start, &end);
        for (widget, _) in self.lenses.borrow_mut().drain(..) {
            self.view.remove(&widget);
        }
        for (widget, line) in &lenses {
            if let Some(from) = buffer.iter_at_line(*line) {
                let mut to = from;
                to.forward_to_line_end();
                buffer.apply_tag(&self.tags.lens, &from, &to);
            }
            self.view.add_overlay(widget, 0, 0);
        }
        *self.lenses.borrow_mut() = lenses;
        self.place_overlays();
    }

    /// Shows a peek under a line (from 0), or none.
    pub fn set_peek(&self, peek: Option<(gtk::Widget, i32)>) {
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.peek, &start, &end);
        if let Some((widget, _)) = self.peek.borrow_mut().take() {
            self.view.remove(&widget);
        }
        if let Some((widget, _)) = &peek {
            self.view.add_overlay(widget, 0, 0);
        }
        *self.peek.borrow_mut() = peek;
        self.size_peek();
        self.place_overlays();
    }

    /// `true` while a peek is open.
    pub fn has_peek(&self) -> bool {
        self.peek.borrow().is_some()
    }

    /// Fits the peek to the editor's width and makes room for it above the
    /// next line, so the cursor's line keeps its own tint.
    fn size_peek(&self) {
        let peek = self.peek.borrow();
        let Some((widget, line)) = peek.as_ref() else {
            return;
        };
        let width = self
            .root
            .width()
            .saturating_sub(GUTTER_WIDTH)
            .saturating_sub(16)
            .max(400);
        if widget.width_request() != width {
            widget.set_size_request(width, -1);
        }
        let (_, natural, _, _) = widget.measure(gtk::Orientation::Vertical, width);
        let next = line.saturating_add(1);
        let lens_below = self.lenses.borrow().iter().any(|(_, l)| *l == next);
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
                if let Some(from) = buffer.iter_at_line(*line) {
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
        for (widget, line) in self.lenses.borrow().iter() {
            if let Some(iter) = buffer.iter_at_line(*line) {
                // Just above the line's text, whatever room is above it.
                let text_top = self.view.iter_location(&iter).y();
                let y = text_top
                    .saturating_sub(LINE_SPACING)
                    .saturating_sub(LENS_HEIGHT)
                    .saturating_add(2);
                self.view.move_overlay(widget, 2, y);
            }
        }
        if let Some((widget, line)) = self.peek.borrow().as_ref()
            && let Some(iter) = buffer.iter_at_line(*line)
        {
            let location = self.view.iter_location(&iter);
            let y = location
                .y()
                .saturating_add(location.height())
                .saturating_add(LINE_SPACING)
                .saturating_add(4);
            self.view.move_overlay(widget, 0, y);
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
