//! The code editor (V2C_EditorOpen.dc.html): a GtkSourceView
//! with line numbers, `.stave` colours from the navigation layer's
//! semantic tokens, wavy underlines for problems, the uses of the name
//! under the cursor marked, lenses above declarations ("2 uses · show in
//! 3D"), an inline peek of a definition under a line, and Ctrl+F search.
//!
//! It only displays: what to colour, underline, mark and show comes from
//! the Files page (`files.rs`), which asks `stave-ide`. Positions arrive
//! as byte offsets and are turned into lines and byte indexes here
//! ([`LineStarts`]), the only place that knows GTK counts in characters.
//! This is also the only file that knows the editor is a GtkSourceView: the
//! page sees a `gtk::TextView` and `gtk::TextBuffer`s.
//!
//! GtkSourceView does what it is good at: the line numbers, the cursor's
//! line, bracket matching, indentation (two spaces, as `stave fmt` lays a
//! file out), smart Home/End, undo and search. Colours do **not** come
//! from a GtkSourceView language definition: a `.lang` file would be a
//! second list of the grammar's words that could drift from it, and it
//! couldn't tell a part's name from a recipe's. The buffer's own syntax
//! highlighting is off; the page paints the semantic tokens as tags
//! ([`Editor::paint`]), so `stave-lang` and `stave-schema` stay the one
//! source of truth. The style scheme (Adwaita or Adwaita-dark, following
//! libadwaita, with the design's tint for the cursor's line) colours
//! everything else: the text, the gutter, matched brackets, search matches.
//!
//! Lenses and the peek are overlay children placed in the room a tag's
//! `pixels-above-lines` / `pixels-below-lines` makes for them.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;
use stave_ide::{SymbolId, SymbolKind, TokenClass};
use stave_lang::Severity;

/// The text's own spacing above and below each line: 13 px type on
/// 23 px lines, as the design has it.
const LINE_SPACING: i32 = 3;

/// The room a lens takes above its line.
const LENS_HEIGHT: i32 = 20;

/// The width of the gutter's strip for the cursor's bar and problems'
/// bars, left of the line numbers.
const MARKS_WIDTH: i32 = 10;

/// Spaces per indentation step: `stave fmt` indents two spaces per open
/// `{` (rule 1 of `stave_lang::format`).
const INDENT_WIDTH: usize = 2;

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

/// How many frames a scroll waits for the text to be given its size.
const SCROLL_WAIT_FRAMES: u32 = 120;

/// A scrolled view's vertical extent, in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Page {
    /// The top of what shows.
    value: f64,
    /// How much shows.
    size: f64,
    /// The smallest top.
    lower: f64,
    /// The bottom of everything.
    upper: f64,
}

impl Page {
    /// `true` once the view has a height to show text in.
    fn is_laid_out(self) -> bool {
        self.size > 1.0
    }

    /// The top that shows the span from `top` to `bottom`, about a third
    /// of the way down, or `None` when it already shows (or there is no
    /// page yet to scroll).
    fn scroll_to_show(self, top: f64, bottom: f64) -> Option<f64> {
        if !self.is_laid_out() {
            return None;
        }
        if top >= self.value && bottom <= self.value + self.size {
            return None;
        }
        let highest = (self.upper - self.size).max(self.lower);
        Some((top - self.size * 0.3).clamp(self.lower, highest))
    }
}

/// The byte offset of an iterator.
pub fn offset_of(lines: &LineStarts, iter: &gtk::TextIter) -> u32 {
    lines.offset(iter.line(), iter.line_index())
}

/// How wide a line's indentation is, in spaces (a tab counts as a step).
fn indentation(line: &str) -> usize {
    line.chars()
        .map_while(|c| match c {
            ' ' => Some(1),
            '\t' => Some(INDENT_WIDTH),
            _ => None,
        })
        .sum()
}

/// `true` if a line opens a block or a bracket it doesn't close.
fn opens(line: &str) -> bool {
    line.trim_end().ends_with(['{', '(', '['])
}

/// The indentation of a new line typed after `previous`: the same as
/// `previous`, one step in after an opening `{`, `(` or `[`.
pub fn indent_after(previous: &str) -> usize {
    let width = indentation(previous);
    if opens(previous) {
        width.saturating_add(INDENT_WIDTH)
    } else {
        width
    }
}

/// The indentation of a line that starts with a closing `}`, `)` or `]`,
/// after `previous` (the nearest line above it that isn't blank): the
/// opening line's when `previous` opened the block, one step out from
/// `previous` otherwise.
pub fn indent_closing(previous: &str) -> usize {
    let width = indentation(previous);
    if opens(previous) {
        width
    } else {
        width.saturating_sub(INDENT_WIDTH)
    }
}

/// The text of a buffer's line, without its line end.
fn line_text(buffer: &gtk::TextBuffer, line: i32) -> Option<String> {
    let start = buffer.iter_at_line(line)?;
    let mut end = start;
    if !end.ends_line() {
        end.forward_to_line_end();
    }
    Some(buffer.text(&start, &end, false).to_string())
}

/// The style scheme for the light or dark style: Adwaita's, with the
/// design's text colour on libadwaita's view background, its tint and
/// accent colour for the cursor's line and number, and a box round matched
/// brackets (`stave-light` and `stave-dark`, derived from
/// Adwaita and Adwaita-dark), or Adwaita's own if those can't be loaded.
fn style_scheme(dark: bool) -> Option<sourceview5::StyleScheme> {
    thread_local! {
        static LOADED: Cell<bool> = const { Cell::new(false) };
    }
    let manager = sourceview5::StyleSchemeManager::default();
    if !LOADED.replace(true)
        && let Some(folder) = write_style_schemes()
        && let Some(folder) = folder.to_str()
    {
        manager.append_search_path(folder);
    }
    let (ours, adwaita) = if dark {
        ("stave-dark", "Adwaita-dark")
    } else {
        ("stave-light", "Adwaita")
    };
    manager.scheme(ours).or_else(|| manager.scheme(adwaita))
}

/// The derived style schemes, as GtkSourceView reads them.
const STYLE_SCHEMES: [(&str, &str); 2] = [
    (
        "stave-light.xml",
        r##"<?xml version="1.0" encoding="UTF-8"?>
<!-- Written by Stave: Adwaita with the editor's colours. -->
<style-scheme id="stave-light" name="Stave" version="1.0" parent-scheme="Adwaita">
  <metadata>
    <property name="variant">light</property>
    <property name="dark-variant">stave-dark</property>
  </metadata>
  <style name="text" foreground="#222226" background="#ffffff"/>
  <style name="current-line" background="#f1f6fd"/>
  <style name="current-line-number" background="#f1f6fd" foreground="#1c71d8" bold="true"/>
  <style name="bracket-match" background="#deddda" bold="true"/>
</style-scheme>
"##,
    ),
    (
        "stave-dark.xml",
        r##"<?xml version="1.0" encoding="UTF-8"?>
<!-- Written by Stave: Adwaita-dark with the editor's colours. -->
<style-scheme id="stave-dark" name="Stave Dark" version="1.0" parent-scheme="Adwaita-dark">
  <metadata>
    <property name="variant">dark</property>
    <property name="light-variant">stave-light</property>
  </metadata>
  <style name="text" foreground="#ffffff" background="#1d1d20"/>
  <style name="current-line" background="#26303d"/>
  <style name="current-line-number" background="#26303d" foreground="#78aeed" bold="true"/>
  <style name="bracket-match" background="#4a4a50" bold="true"/>
</style-scheme>
"##,
    ),
];

/// Writes the derived style schemes into the user's cache folder
/// (GtkSourceView loads schemes only from folders), unless they are there
/// already; the folder, or `None` if it can't be written.
fn write_style_schemes() -> Option<std::path::PathBuf> {
    let folder = glib::user_cache_dir().join("stave").join("styles");
    std::fs::create_dir_all(&folder).ok()?;
    for (name, text) in STYLE_SCHEMES {
        let path = folder.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
            std::fs::write(&path, text).ok()?;
        }
    }
    Some(folder)
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
    }
}

mod imp {
    use super::*;

    /// The strip of the gutter left of the line numbers: the accent bar of
    /// the cursor's line and a bar beside each line with a problem.
    #[derive(Default)]
    pub struct Marks {
        pub dark: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Marks {
        const NAME: &'static str = "StaveGutterMarks";
        type Type = super::Marks;
        type ParentType = sourceview5::GutterRenderer;
    }

    impl ObjectImpl for Marks {}

    impl WidgetImpl for Marks {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                gtk::Orientation::Horizontal => (MARKS_WIDTH, MARKS_WIDTH, -1, -1),
                _ => (0, 0, -1, -1),
            }
        }
    }

    impl GutterRendererImpl for Marks {
        fn snapshot_line(
            &self,
            snapshot: &gtk::Snapshot,
            lines: &sourceview5::GutterLines,
            line: u32,
        ) {
            let (y, height) =
                lines.line_yrange(line, sourceview5::GutterRendererAlignmentMode::Cell);
            let view = lines.view();
            let dark = self.dark.get();
            if lines.is_cursor(line) {
                let accent = gtk::gdk::RGBA::parse(if dark { "#78aeed" } else { "#1c71d8" })
                    .unwrap_or(gtk::gdk::RGBA::BLUE);
                let bar = gtk::graphene::Rect::new(
                    0.0,
                    crate::convert::f32_of_int(y),
                    3.0,
                    crate::convert::f32_of_int(height),
                );
                snapshot.append_color(&accent, &bar);
            }
            let start = lines.iter_at_line(line);
            if let Some(severity) = line_problem(&view, &start) {
                // Beside the text, below any lens or peek above it.
                let text = view.iter_location(&start).height();
                let bottom = y.saturating_add(height).saturating_sub(LINE_SPACING);
                let bar = gtk::graphene::Rect::new(
                    5.0,
                    crate::convert::f32_of_int(bottom.saturating_sub(text)),
                    3.0,
                    crate::convert::f32_of_int(text),
                );
                snapshot.append_color(&problem_colour(severity, dark), &bar);
            }
        }
    }

    /// The text view: a `GtkSourceView` that draws problems' wavy
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
        type ParentType = sourceview5::View;
    }

    impl ObjectImpl for CodeView {}
    impl WidgetImpl for CodeView {}
    impl ViewImpl for CodeView {}

    impl TextViewImpl for CodeView {
        fn snapshot_layer(&self, layer: gtk::TextViewLayer, snapshot: gtk::Snapshot) {
            self.parent_snapshot_layer(layer, snapshot.clone());
            if layer != gtk::TextViewLayer::AboveText {
                return;
            }
            let view = self.obj();
            let view: &gtk::TextView = view.upcast_ref();
            let table = view.buffer().tag_table();
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

    /// Indents a new line as `stave fmt` would: one step in after an
    /// opening bracket, and a closing bracket typed first on a line one
    /// step back out.
    #[derive(Default)]
    pub struct BlockIndenter;

    #[glib::object_subclass]
    impl ObjectSubclass for BlockIndenter {
        const NAME: &'static str = "StaveBlockIndenter";
        type Type = super::BlockIndenter;
        type ParentType = glib::Object;
        type Interfaces = (sourceview5::Indenter,);
    }

    impl ObjectImpl for BlockIndenter {}

    impl IndenterImpl for BlockIndenter {
        fn is_trigger(
            &self,
            _view: &sourceview5::View,
            _location: &gtk::TextIter,
            state: gtk::gdk::ModifierType,
            keyval: gtk::gdk::Key,
        ) -> bool {
            let plain = !state.intersects(
                gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK,
            );
            plain
                && matches!(
                    keyval,
                    gtk::gdk::Key::Return
                        | gtk::gdk::Key::KP_Enter
                        | gtk::gdk::Key::ISO_Enter
                        | gtk::gdk::Key::braceright
                        | gtk::gdk::Key::parenright
                        | gtk::gdk::Key::bracketright
                )
        }

        fn indent(&self, _view: &sourceview5::View, iter: &mut gtk::TextIter) {
            let buffer = iter.buffer();
            let line = iter.line();
            if iter.starts_line() {
                // A new line: indent it after the line above.
                let Some(previous) = line_text(&buffer, line.saturating_sub(1)) else {
                    return;
                };
                if line > 0 {
                    buffer.insert(iter, &" ".repeat(indent_after(&previous)));
                }
                return;
            }
            // A closing bracket: if it is the first thing on its line, line
            // it up with the line that opened the block.
            let mut before = *iter;
            before.backward_char();
            let Some(mut start) = buffer.iter_at_line(line) else {
                return;
            };
            if !buffer.text(&start, &before, false).trim().is_empty() {
                return;
            }
            let previous = (0..line)
                .rev()
                .filter_map(|l| line_text(&buffer, l))
                .find(|text| !text.trim().is_empty());
            let Some(previous) = previous else {
                return;
            };
            let width = indent_closing(&previous);
            buffer.delete(&mut start, &mut before);
            buffer.insert(&mut start, &" ".repeat(width));
            *iter = start;
            iter.forward_char();
        }
    }
}

glib::wrapper! {
    /// The gutter's strip of bars: the cursor's line and problems.
    pub struct Marks(ObjectSubclass<imp::Marks>)
        @extends sourceview5::GutterRenderer, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

glib::wrapper! {
    /// The editor's text view.
    pub struct CodeView(ObjectSubclass<imp::CodeView>)
        @extends sourceview5::View, gtk::TextView, gtk::Widget,
        @implements gtk::Accessible, gtk::AccessibleText, gtk::Buildable, gtk::ConstraintTarget, gtk::Scrollable;
}

glib::wrapper! {
    /// Indents as `stave fmt` lays a file out.
    pub struct BlockIndenter(ObjectSubclass<imp::BlockIndenter>)
        @implements sourceview5::Indenter;
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

/// A coloured run, as lines and byte indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// Its first byte.
    pub start: u32,
    /// One past its last byte.
    pub end: u32,
}

/// The search bar over the text (Ctrl+F in the editor), and the search of
/// the buffer showing.
struct Search {
    revealer: gtk::Revealer,
    entry: gtk::SearchEntry,
    count: gtk::Label,
    settings: sourceview5::SearchSettings,
    /// The search of the buffer showing, while the bar is open.
    context: RefCell<Option<sourceview5::SearchContext>>,
    view: glib::WeakRef<gtk::TextView>,
}

impl Search {
    fn new(view: &gtk::TextView) -> Rc<Search> {
        let entry = gtk::SearchEntry::builder()
            .placeholder_text("Find in this file")
            .width_chars(22)
            .build();
        let count = gtk::Label::builder()
            .css_classes(["dim-label", "numeric"])
            .width_chars(9)
            .xalign(1.0)
            .build();
        let button = |icon: &str, tooltip: &str| {
            gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tooltip)
                .css_classes(["flat", "circular"])
                .valign(gtk::Align::Center)
                .build()
        };
        let previous = button("go-up-symbolic", "Previous match (Shift+Enter)");
        let next = button("go-down-symbolic", "Next match (Enter)");
        let close = button("window-close-symbolic", "Close the search (Escape)");
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .css_classes(["card"])
            .build();
        for margin in [
            gtk::Widget::set_margin_top,
            gtk::Widget::set_margin_bottom,
            gtk::Widget::set_margin_start,
            gtk::Widget::set_margin_end,
        ] {
            margin(entry.upcast_ref(), 6);
        }
        entry.set_margin_end(0);
        close.set_margin_end(6);
        bar.append(&entry);
        bar.append(&count);
        bar.append(&previous);
        bar.append(&next);
        bar.append(&close);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_end(18)
            .margin_top(6)
            .child(&bar)
            .reveal_child(false)
            .build();
        // Hidden while closed, so it takes no pointer events.
        revealer.set_visible(false);
        let settings = sourceview5::SearchSettings::builder()
            .wrap_around(true)
            .case_sensitive(false)
            .build();
        let search = Rc::new(Search {
            revealer,
            entry,
            count,
            settings,
            context: RefCell::new(None),
            view: view.downgrade(),
        });
        search.connect(&previous, &next, &close);
        search
    }

    fn connect(self: &Rc<Self>, previous: &gtk::Button, next: &gtk::Button, close: &gtk::Button) {
        let weak = Rc::downgrade(self);
        let with = move |action: fn(&Search)| {
            let weak: Weak<Search> = weak.clone();
            move || {
                if let Some(search) = weak.upgrade() {
                    action(&search);
                }
            }
        };
        let changed = with(Search::text_changed);
        self.entry.connect_search_changed(move |_| changed());
        let forward = with(Search::next);
        self.entry.connect_activate(move |_| forward());
        let forward = with(Search::next);
        self.entry.connect_next_match(move |_| forward());
        let backward = with(Search::previous);
        self.entry.connect_previous_match(move |_| backward());
        let stop = with(Search::close);
        self.entry.connect_stop_search(move |_| stop());
        let forward = with(Search::next);
        next.connect_clicked(move |_| forward());
        let backward = with(Search::previous);
        previous.connect_clicked(move |_| backward());
        let stop = with(Search::close);
        close.connect_clicked(move |_| stop());
        // Shift+Enter goes back.
        let keys = gtk::EventControllerKey::new();
        let backward = with(Search::previous);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            if shift && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
                backward();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.entry.add_controller(keys);
    }

    fn is_open(&self) -> bool {
        self.revealer.reveals_child()
    }

    /// Opens the bar with the selection (one line of it) as the text to
    /// find, or the last one.
    fn open(&self) {
        let Some(view) = self.view.upgrade() else {
            return;
        };
        let buffer = view.buffer();
        if let Some((start, end)) = buffer.selection_bounds()
            && start.line() == end.line()
        {
            self.entry.set_text(&buffer.text(&start, &end, false));
        }
        self.revealer.set_visible(true);
        self.revealer.set_reveal_child(true);
        self.attach();
        self.settings
            .set_search_text(Some(self.entry.text().as_str()).filter(|t| !t.is_empty()));
        self.refresh_count();
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
    }

    /// Opens the bar and finds `text` from the cursor at once.
    fn find(&self, text: &str) {
        self.open();
        self.entry.set_text(text);
        self.text_changed();
    }

    fn close(&self) {
        if !self.is_open() {
            return;
        }
        self.revealer.set_reveal_child(false);
        self.revealer.set_visible(false);
        self.settings.set_search_text(None);
        self.context.replace(None);
        if let Some(view) = self.view.upgrade() {
            view.grab_focus();
        }
    }

    /// Searches the buffer showing, if the bar is open.
    fn attach(&self) {
        let buffer = self
            .view
            .upgrade()
            .and_then(|view| view.buffer().downcast::<sourceview5::Buffer>().ok());
        let context = buffer.filter(|_| self.is_open()).map(|buffer| {
            let context = sourceview5::SearchContext::new(&buffer, Some(&self.settings));
            context.set_highlight(true);
            let search = self.view.clone();
            let count = self.count.downgrade();
            let entry = self.entry.downgrade();
            context.connect_occurrences_count_notify(move |context| {
                if let (Some(view), Some(count), Some(entry)) =
                    (search.upgrade(), count.upgrade(), entry.upgrade())
                {
                    count.set_label(&count_text(context, &view.buffer(), &entry.text()));
                }
            });
            context
        });
        self.context.replace(context);
    }

    /// The text to find changed: find it from the start of the selection,
    /// so a longer text keeps the match it had.
    fn text_changed(&self) {
        let text = self.entry.text();
        self.settings
            .set_search_text(Some(text.as_str()).filter(|t| !t.is_empty()));
        if !text.is_empty() {
            self.step(true, true);
        }
        self.refresh_count();
    }

    fn next(&self) {
        self.step(true, false);
    }

    fn previous(&self) {
        self.step(false, false);
    }

    /// Selects the next (or previous) match and scrolls to it; `again`
    /// starts from the selection's start, to keep the match there.
    fn step(&self, forward: bool, again: bool) {
        let (Some(view), Some(context)) = (self.view.upgrade(), self.context.borrow().clone())
        else {
            return;
        };
        let buffer = view.buffer();
        let (start, end) = buffer.selection_bounds().unwrap_or_else(|| {
            let cursor = buffer.iter_at_mark(&buffer.get_insert());
            (cursor, cursor)
        });
        let found = match (forward, again) {
            (true, true) => context.forward(&start),
            (true, false) => context.forward(&end),
            (false, _) => context.backward(&start),
        };
        if let Some((from, to, _)) = found {
            buffer.select_range(&from, &to);
            view.scroll_to_mark(&buffer.get_insert(), 0.2, false, 0.0, 0.0);
        }
        self.refresh_count();
    }

    fn refresh_count(&self) {
        let (Some(view), Some(context)) = (self.view.upgrade(), self.context.borrow().clone())
        else {
            self.count.set_label("");
            return;
        };
        self.count
            .set_label(&count_text(&context, &view.buffer(), &self.entry.text()));
    }
}

/// "3 of 12", "12 matches", "No matches", or nothing while counting.
fn count_text(
    context: &sourceview5::SearchContext,
    buffer: &gtk::TextBuffer,
    text: &str,
) -> String {
    let total = context.occurrences_count();
    if text.is_empty() || total < 0 {
        return String::new();
    }
    if total == 0 {
        return "No matches".to_owned();
    }
    let position = buffer
        .selection_bounds()
        .map_or(0, |(start, end)| context.occurrence_position(&start, &end));
    match (position, total) {
        (1.., _) => format!("{position} of {total}"),
        (_, 1) => "1 match".to_owned(),
        _ => format!("{total} matches"),
    }
}

/// The editor widget.
pub struct Editor {
    /// The editor, ready to pack: the text in a scrolled window, with the
    /// search bar over it.
    pub root: gtk::Overlay,
    /// The text view.
    pub view: gtk::TextView,
    code: CodeView,
    scrolled: gtk::ScrolledWindow,
    table: gtk::TextTagTable,
    tags: Tags,
    marks: Marks,
    search: Rc<Search>,
    dark: Cell<bool>,
    /// The lens rows, made as needed and kept (GTK can't take an overlay
    /// back out of a text view, so they are hidden when not used).
    lenses: RefCell<Vec<LensSlot>>,
    /// How many of the lens rows are in use.
    lenses_used: Cell<usize>,
    /// Where the peek goes, and the line it sits under while it shows.
    peek_box: gtk::Box,
    peek_line: Cell<Option<i32>>,
    /// The line and room the peek's tag was last applied for: applied
    /// again only when they change, since every change lays the text out
    /// again.
    peek_tagged: Cell<Option<(i32, i32)>>,
    /// Where the hover card goes.
    card_box: gtk::Box,
    /// Counts go-tos, so a scroll still waiting for the text's size gives
    /// way to a newer one.
    scroll_ticket: Rc<Cell<u64>>,
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
        let code: CodeView = glib::Object::builder::<CodeView>()
            .property("monospace", true)
            .property("wrap-mode", gtk::WrapMode::None)
            .property("top-margin", 12)
            .property("bottom-margin", 48)
            .property("left-margin", 2)
            .property("right-margin", 16)
            .property("pixels-above-lines", LINE_SPACING)
            .property("pixels-below-lines", LINE_SPACING)
            .property("show-line-numbers", true)
            .property("highlight-current-line", true)
            .property("auto-indent", true)
            .property("indent-width", 2)
            .property("tab-width", 2u32)
            .property("insert-spaces-instead-of-tabs", true)
            .property("indent-on-tab", true)
            .property("smart-backspace", true)
            .property("smart-home-end", sourceview5::SmartHomeEndType::Before)
            .property("css-classes", ["stave-editor"].to_value())
            .build();
        code.set_indenter(Some(&glib::Object::new::<BlockIndenter>()));
        let view: gtk::TextView = code.clone().upcast();
        let editor_buffer = sourceview5::Buffer::new(Some(&table));
        editor_buffer.set_highlight_syntax(false);
        view.set_buffer(Some(&editor_buffer));
        let gutter = sourceview5::prelude::ViewExt::gutter(&code, gtk::TextWindowType::Left);
        let marks: Marks = glib::Object::builder().build();
        // Left of the line numbers (GtkSourceView puts them at -30).
        gutter.insert(&marks, -40);
        // The line numbers sit level with their text, below any lens, and
        // a little way from it, as in the design.
        let mut child = gutter.first_child();
        while let Some(widget) = child {
            if let Some(numbers) = widget.downcast_ref::<sourceview5::GutterRendererText>() {
                numbers.set_yalign(1.0);
                numbers.set_ypad(1);
                numbers.set_xpad(12);
            }
            child = widget.next_sibling();
        }
        let scrolled = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .hexpand(true)
            .build();
        // A wider editor scrolls back to the start of the lines when the
        // cursor fits without scrolling.
        let text = view.downgrade();
        scrolled.hadjustment().connect_changed(move |horizontal| {
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
        let search = Search::new(&view);
        let root = gtk::Overlay::builder().child(&scrolled).build();
        root.add_overlay(&search.revealer);
        let editor = Editor {
            root,
            view,
            code,
            scrolled,
            table,
            tags,
            marks,
            search,
            dark: Cell::new(false),
            lenses: RefCell::new(Vec::new()),
            lenses_used: Cell::new(0),
            peek_box,
            peek_line: Cell::new(None),
            peek_tagged: Cell::new(None),
            card_box,
            scroll_ticket: Rc::new(Cell::new(0)),
            on_uses: Rc::new(RefCell::new(None)),
            on_show: Rc::new(RefCell::new(None)),
        };
        editor.colour(adw::StyleManager::default().is_dark());
        editor
    }

    /// The text's vertical scrolling.
    pub fn vadjustment(&self) -> gtk::Adjustment {
        self.scrolled.vadjustment()
    }

    /// A new buffer for a document, sharing the editor's tags. Its text
    /// can't be undone back to empty.
    pub fn new_buffer(&self, text: &str) -> gtk::TextBuffer {
        let buffer = sourceview5::Buffer::new(Some(&self.table));
        buffer.set_highlight_syntax(false);
        buffer.set_highlight_matching_brackets(true);
        buffer.begin_irreversible_action();
        buffer.set_text(text);
        buffer.end_irreversible_action();
        buffer.place_cursor(&buffer.start_iter());
        buffer.upcast()
    }

    /// Shows a buffer; read-only documents can't be typed in.
    pub fn show_buffer(&self, buffer: &gtk::TextBuffer, editable: bool) {
        self.clear_overlays();
        if let Some(source) = buffer.downcast_ref::<sourceview5::Buffer>() {
            source.set_style_scheme(style_scheme(self.dark.get()).as_ref());
        }
        self.view.set_buffer(Some(buffer));
        self.view.set_editable(editable);
        self.view.set_cursor_visible(true);
        if self.search.is_open() {
            self.search.attach();
            self.search.refresh_count();
        }
        self.marks.queue_draw();
    }

    /// Follows the light or dark style.
    pub fn colour(&self, dark: bool) {
        self.dark.set(dark);
        self.tags.colour(dark);
        if let Ok(source) = self.view.buffer().downcast::<sourceview5::Buffer>() {
            source.set_style_scheme(style_scheme(dark).as_ref());
        }
        self.code.imp().dark.set(dark);
        self.view.queue_draw();
        self.marks.imp().dark.set(dark);
        self.marks.queue_draw();
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
        self.marks.queue_draw();
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

    /// `true` while the keyboard is in the text or its search bar.
    pub fn has_focus(&self) -> bool {
        self.view.has_focus()
            || self
                .view
                .root()
                .and_then(|root| root.focus())
                .is_some_and(|focus| focus.is_ancestor(&self.search.revealer))
    }

    /// Opens the search bar (Ctrl+F in the text).
    pub fn start_search(&self) {
        self.search.open();
    }

    /// Opens the search bar and finds `text` from the cursor.
    pub fn find(&self, text: &str) {
        self.search.find(text);
    }

    /// `true` while the search bar is open.
    pub fn has_search(&self) -> bool {
        self.search.is_open()
    }

    /// Closes the search bar and its highlights.
    pub fn close_search(&self) {
        self.search.close();
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
        let uses = super::style::as_link(
            gtk::Button::builder()
                .css_classes(["flat", "stave-lens-link"])
                .build(),
        );
        let dot = gtk::Label::new(Some("·"));
        dot.add_css_class("stave-lens-dot");
        let show = super::style::as_link(
            gtk::Button::builder()
                .label("show in 3D")
                .css_classes(["flat", "stave-lens-link"])
                .build(),
        );
        root.append(&uses);
        root.append(&dot);
        root.append(&show);
        let symbol: Rc<Cell<Option<SymbolId>>> = Rc::new(Cell::new(None));
        for (button, callbacks) in [(&uses, &self.on_uses), (&show, &self.on_show)] {
            let call = {
                let symbol = symbol.clone();
                let callbacks = callbacks.clone();
                Rc::new(move || {
                    let callback = callbacks.borrow().clone();
                    if let (Some(callback), Some(symbol)) = (callback, symbol.get()) {
                        callback(symbol);
                    }
                })
            };
            // From the keyboard.
            let keyboard = call.clone();
            button.connect_clicked(move |_| keyboard());
            // A button doesn't claim the press, so the text view under the
            // lens would take it too and start selecting text. This claims
            // it first (which also keeps the button's own click from
            // firing) and calls on release.
            let mouse = gtk::GestureClick::builder()
                .button(gtk::gdk::BUTTON_PRIMARY)
                .propagation_phase(gtk::PropagationPhase::Capture)
                .build();
            mouse.connect_pressed(|gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            });
            mouse.connect_released(move |_, _, _, _| call());
            button.add_controller(mouse);
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
        self.peek_tagged.set(None);
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
        let gutter = sourceview5::prelude::ViewExt::gutter(&self.code, gtk::TextWindowType::Left);
        let width = self
            .scrolled
            .width()
            .saturating_sub(gutter.width())
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
        if self.peek_tagged.get() == Some((line, room)) {
            return;
        }
        self.peek_tagged.set(Some((line, room)));
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
    ///
    /// The scroll waits until the text has been given its size: a file
    /// opened into a panel that was showing "No file open" (or one behind
    /// another tab) has no height yet and its lines have no real places,
    /// so a scroll worked out then lands anywhere.
    pub fn go_to(&self, lines: &LineStarts, offset: u32) {
        let buffer = self.view.buffer();
        let iter = iter_at(&buffer, lines, offset);
        buffer.place_cursor(&iter);
        let mark = buffer.get_insert();
        // A newer go-to replaces one still waiting.
        let ticket = self.scroll_ticket.get().wrapping_add(1);
        self.scroll_ticket.set(ticket);
        let tickets = Rc::clone(&self.scroll_ticket);
        let scrolled = self.scrolled.downgrade();
        let frames = Cell::new(0u32);
        // Ticks come once a frame, and not while the view is off screen.
        self.view.add_tick_callback(move |view, _| {
            let Some(scrolled) = scrolled.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if tickets.get() != ticket || mark.buffer().as_ref() != Some(&view.buffer()) {
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get().saturating_add(1));
            let vertical = scrolled.vadjustment();
            let page = Page {
                value: vertical.value(),
                size: vertical.page_size(),
                lower: vertical.lower(),
                upper: vertical.upper(),
            };
            if !page.is_laid_out() || view.height() <= 1 {
                return if frames.get() < SCROLL_WAIT_FRAMES {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                };
            }
            let cursor = view.buffer().iter_at_mark(&mark);
            let location = view.iter_location(&cursor);
            if let Some(value) = page.scroll_to_show(
                f64::from(location.y()),
                f64::from(location.y().saturating_add(location.height())),
            ) {
                vertical.set_value(value);
            }
            // Up and down only, so the line keeps its start in view,
            // unless the cursor is off to the right.
            let horizontal = scrolled.hadjustment();
            let right = f64::from(location.x().saturating_add(24));
            let width = horizontal.page_size();
            if width > 50.0 {
                horizontal.set_value(if right > width { right - width } else { 0.0 });
            }
            glib::ControlFlow::Break
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
    fn a_view_with_no_height_yet_is_not_scrolled() {
        // A file opened into a panel that was showing "No file open": the
        // text has no height, and its line places are not real yet.
        let no_height = Page {
            value: 0.0,
            size: 0.0,
            lower: 0.0,
            upper: 556.0,
        };
        assert!(!no_height.is_laid_out());
        assert_eq!(no_height.scroll_to_show(215.0, 233.0), None);
    }

    #[test]
    fn a_line_already_showing_stays_where_it_is() {
        let page = Page {
            value: 0.0,
            size: 703.0,
            lower: 0.0,
            upper: 703.0,
        };
        assert_eq!(page.scroll_to_show(199.0, 215.0), None);
    }

    #[test]
    fn a_line_out_of_sight_comes_a_third_of_the_way_down() {
        let page = Page {
            value: 0.0,
            size: 300.0,
            lower: 0.0,
            upper: 2000.0,
        };
        assert_eq!(page.scroll_to_show(1000.0, 1020.0), Some(910.0));
        // Never past the end of the text.
        assert_eq!(page.scroll_to_show(1990.0, 2000.0), Some(1700.0));
        // Nor above its start.
        let scrolled = Page {
            value: 500.0,
            ..page
        };
        assert_eq!(scrolled.scroll_to_show(20.0, 40.0), Some(0.0));
    }

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

    #[test]
    fn a_new_line_is_indented_one_step_inside_a_block() {
        assert_eq!(indent_after("part side_l : oak_19 {"), 2);
        assert_eq!(indent_after("  length: 600 mm"), 2);
        assert_eq!(indent_after("  joint dado(  "), 4);
        assert_eq!(indent_after(""), 0);
        assert_eq!(indent_after("\tx {"), 4);
    }

    #[test]
    fn a_closing_bracket_goes_back_to_the_line_that_opened_it() {
        assert_eq!(indent_closing("  length: 600 mm"), 0);
        assert_eq!(indent_closing("    part a : b {"), 4);
        assert_eq!(indent_closing("x"), 0);
    }

    /// The editor's indentation agrees with `stave fmt` on every line
    /// right after an opening `{` and every `}` after a field, in the
    /// formatted examples.
    #[test]
    fn indentation_matches_stave_fmt() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let mut files = Vec::new();
        let mut folders = vec![root];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.extension().is_some_and(|e| e == "stave") {
                    files.push(path);
                }
            }
        }
        assert!(!files.is_empty());
        let mut checked = 0;
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            let mut sources = stave_lang::SourceMap::new();
            let file = sources
                .add(path.display().to_string(), text.as_str())
                .unwrap();
            let formatted = stave_lang::format_file(file, &text).unwrap();
            let lines: Vec<&str> = formatted.lines().collect();
            for pair in lines.windows(2) {
                let [previous, line] = pair else { continue };
                if line.trim().is_empty() || !previous.trim_end().ends_with('{') {
                    continue;
                }
                let wanted = if line.trim_start().starts_with('}') {
                    indent_closing(previous)
                } else {
                    indent_after(previous)
                };
                assert_eq!(indentation(line), wanted, "{}: {line:?}", path.display());
                checked += 1;
            }
        }
        assert!(checked > 50, "only {checked} lines checked");
    }
}
