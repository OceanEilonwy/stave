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
//! `pixels-above-lines` / `pixels-below-lines` makes for them; the room is
//! as tall as they are (a lens row grows with Large Text), and they follow
//! the text's layout for a while after each change. So are the links above
//! a conflict ("Keep yours · Keep both"), which [`Editor::paint`] puts
//! there while it tints the conflict's sides. A conflict link is the one
//! widget here that changes something itself: it edits the buffer, as
//! typing does, and the page hears the change as it hears typing.
//!
//! The search bar is a relm4 component, [`EditorSearch`]
//! (docs/relm4-migration.md), whose controller the editor owns: what it
//! is told is queued, and its widgets only send messages. It never
//! touches the text: its outputs, and the search's counts, come back to
//! the editor as [`SearchEvent`]s on the sender given to [`Editor::new`],
//! which the component forwards into its own input and hands back to
//! [`Editor::search_event`]; the editor holds the search of the buffer,
//! selects the matches, gives the text the keyboard and tells the bar the
//! count. The rest of
//! the editor is the view of the Files page's component
//! (`files::FilesComponent`): its model owns the [`Editor`] and only its
//! `update` calls it, so [`Editor::show_buffer`] and [`Editor::go_to`]
//! take effect at once inside that update, which reads the cursor straight
//! after. The editor's own widgets only send: a lens's links send a
//! [`LensClick`] on the sender given to [`Editor::new`], whose receiver
//! the component forwards into its own input, each lens's handlers made
//! again for the declaration it shows; Escape asks an [`EscapeProbe`] (widgets only) whether anything
//! is open to close. Its state is plain fields of that one owner, changed
//! only through `&mut self`; the two GObject subclasses that draw in the
//! light or dark style read it from their own CSS class.

use stave_app::i18n::{fill_in, ntr, tr};

use adw::prelude::*;
use gtk::glib;
use gtk::subclass::prelude::*;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use sourceview5::prelude::*;
use sourceview5::subclass::prelude::*;
use stave_ide::{SymbolId, TokenClass, TokenModifiers};
use stave_lang::Severity;

use super::theme::{self, Role, Slant, Weight};

/// The CSS class the text view and its gutter's marks carry in the dark
/// style: they read it when they draw.
const DARK_CLASS: &str = "stave-code-dark";

/// The text's own spacing above and below each line: 13 px type on
/// 23 px lines, as the design has it.
const LINE_SPACING: i32 = 3;

/// The least room a lens takes above its line; more when its row is
/// taller (larger text): see [`Editor::lens_room`].
const LENS_HEIGHT: i32 = 20;

/// The room a lens row keeps free below it, above its line's own spacing,
/// and above it.
const LENS_GAP: i32 = 2;

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
    // The schemes are written and their folder searched once a process.
    static LOADED: std::sync::Once = std::sync::Once::new();
    let manager = sourceview5::StyleSchemeManager::default();
    LOADED.call_once(|| {
        if let Some(folder) = write_style_schemes()
            && let Some(folder) = folder.to_str()
        {
            manager.append_search_path(folder);
        }
    });
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
    /// One tag per kind of word, in the theme's order (`theme.rs`).
    syntax: Vec<(Role, gtk::TextTag)>,
    /// The underline of a name that means an item declared elsewhere.
    reference: gtk::TextTag,
    error: gtk::TextTag,
    warning: gtk::TextTag,
    declared: gtk::TextTag,
    used: gtk::TextTag,
    lens: gtk::TextTag,
    peek: gtk::TextTag,
    /// A conflict's marker lines (grammar, "Conflicts").
    conflict_marker: gtk::TextTag,
    /// A conflict's first side.
    conflict_side: gtk::TextTag,
    /// A conflict's other sides.
    conflict_other: gtk::TextTag,
    /// What a conflict's sides changed from.
    conflict_base: gtk::TextTag,
    /// The room above a conflict's opening line for its links.
    conflict_links: gtk::TextTag,
}

impl Tags {
    fn new(table: &gtk::TextTagTable) -> Tags {
        let tag = |name: &str| {
            let tag = gtk::TextTag::new(Some(name));
            table.add(&tag);
            tag
        };
        // Tags made later win where they set the same property.
        let used = tag("used");
        let declared = tag("declared");
        let syntax = Role::SYNTAX
            .into_iter()
            .map(|role| {
                let made = tag(role.tag_name());
                let look = role.look();
                match look.slant {
                    Slant::Upright => {}
                    Slant::Italic => made.set_style(gtk::pango::Style::Italic),
                }
                match look.weight {
                    Weight::Regular => {}
                    Weight::Bold => made.set_weight(look.weight.pango()),
                }
                (role, made)
            })
            .collect();
        let reference = tag("reference");
        reference.set_underline(theme::REFERENCE_UNDERLINE);
        let tags = Tags {
            syntax,
            reference,
            used,
            declared,
            error: tag("error"),
            warning: tag("warning"),
            lens: tag("lens"),
            peek: tag("peek"),
            // The marker lines' tint first, so the sides' (added later,
            // so stronger) cover the lines they share with it.
            conflict_marker: tag("conflict-marker"),
            conflict_side: tag("conflict-side"),
            conflict_other: tag("conflict-other"),
            conflict_base: tag("conflict-base"),
            conflict_links: tag("conflict-links"),
        };
        tags.declared.set_weight(Weight::Bold.pango());
        tags.lens
            .set_pixels_above_lines(LINE_SPACING.saturating_add(LENS_HEIGHT));
        tags.conflict_links
            .set_pixels_above_lines(LINE_SPACING.saturating_add(LENS_HEIGHT));
        tags
    }

    /// The tags that show conflicts.
    fn conflict(&self) -> [&gtk::TextTag; 5] {
        [
            &self.conflict_marker,
            &self.conflict_side,
            &self.conflict_other,
            &self.conflict_base,
            &self.conflict_links,
        ]
    }

    /// The tag a word of this class is painted with; a character the
    /// lexer couldn't read takes the error's underline.
    fn class(&self, class: TokenClass) -> Option<&gtk::TextTag> {
        match Role::of_token(class)? {
            Role::Error => Some(&self.error),
            role => self.role(role),
        }
    }

    /// The tag of a role.
    fn role(&self, role: Role) -> Option<&gtk::TextTag> {
        self.syntax
            .iter()
            .find(|(each, _)| *each == role)
            .map(|(_, tag)| tag)
    }

    /// Every tag [`Editor::paint`] sets, to clear before painting again.
    fn syntax(&self) -> impl Iterator<Item = &gtk::TextTag> {
        self.syntax
            .iter()
            .map(|(_, tag)| tag)
            .chain(std::iter::once(&self.reference))
    }

    /// The theme's colours (`theme.rs`) for the light or dark style.
    fn colour(&self, dark: bool) {
        let rgba = |hex: &str| gtk::gdk::RGBA::parse(hex).unwrap_or(gtk::gdk::RGBA::BLACK);
        for (role, tag) in &self.syntax {
            tag.set_foreground_rgba(Some(&rgba(role.look().colour(dark))));
        }
        self.declared
            .set_background_rgba(Some(&rgba(if dark { "#1e3a5f" } else { "#d3e4f9" })));
        self.used
            .set_background_rgba(Some(&rgba(if dark { "#25344a" } else { "#e6eefa" })));
        let tint = |tag: &gtk::TextTag, light: &str, darker: &str| {
            tag.set_paragraph_background_rgba(Some(&rgba(if dark { darker } else { light })));
        };
        tint(&self.conflict_marker, "#deddda", "#3d3846");
        tint(&self.conflict_side, "#e0f5e6", "#1f3a29");
        tint(&self.conflict_other, "#e0ebfa", "#1e3047");
        tint(&self.conflict_base, "#fbf0d9", "#3f3420");
    }
}

mod imp {
    use super::*;

    /// The strip of the gutter left of the line numbers: the accent bar of
    /// the cursor's line and a bar beside each line with a problem.
    #[derive(Default)]
    pub struct Marks;

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
            let dark = self.obj().has_css_class(DARK_CLASS);
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
                let colour = problem_colour(severity, dark);
                for (top, length) in bar_pieces(severity, bottom.saturating_sub(text), text) {
                    let piece = gtk::graphene::Rect::new(
                        5.0,
                        crate::convert::f32_of_int(top),
                        3.0,
                        crate::convert::f32_of_int(length),
                    );
                    snapshot.append_color(&colour, &piece);
                }
            }
        }
    }

    /// The text view: a `GtkSourceView` that draws problems' wavy
    /// underlines itself, thicker than Pango's, in libadwaita's error and
    /// warning colours.
    #[derive(Default)]
    pub struct CodeView;

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
            let dark = view.has_css_class(DARK_CLASS);
            for (name, severity) in [("warning", Severity::Warning), ("error", Severity::Error)] {
                let Some(tag) = table.lookup(name) else {
                    continue;
                };
                let colour = problem_colour(severity, dark);
                draw_waves(view, &tag, severity, &colour, &snapshot);
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
    let role = match severity {
        Severity::Error => Role::Error,
        Severity::Warning | Severity::Note => Role::Warning,
    };
    gtk::gdk::RGBA::parse(role.look().colour(dark)).unwrap_or(gtk::gdk::RGBA::RED)
}

/// Draws a wave under every visible run of `tag`, line by line, in
/// buffer coordinates (the layer's).
fn draw_waves(
    view: &gtk::TextView,
    tag: &gtk::TextTag,
    severity: Severity,
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
                    severity,
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

/// The pieces of a problem's bar in the gutter, from `top`, `length`
/// pixels long, as (top, length): an error's is whole, a warning's is
/// dashed (3 px on, 2 px off), so the two differ without their colour.
fn bar_pieces(severity: Severity, top: i32, length: i32) -> Vec<(i32, i32)> {
    match severity {
        Severity::Error => vec![(top, length)],
        Severity::Warning | Severity::Note => {
            let end = top.saturating_add(length);
            let mut pieces = Vec::new();
            let mut at = top;
            while at < end {
                pieces.push((at, 3.min(end.saturating_sub(at))));
                at = at.saturating_add(5);
            }
            pieces
        }
    }
}

/// A problem's line from `x0` to `x1` around `y`: for an error, wavy,
/// 1.6 px thick, 1.6 px either side of the line, a crest every 5 px; for
/// a warning, dotted, so the two differ without their colour.
fn wave(
    snapshot: &gtk::Snapshot,
    severity: Severity,
    x0: f64,
    x1: f64,
    y: f64,
    colour: &gtk::gdk::RGBA,
) {
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
    if matches!(severity, Severity::Warning | Severity::Note) {
        context.set_line_width(2.0);
        context.set_line_cap(gtk::cairo::LineCap::Round);
        context.set_dash(&[0.0, 4.0], 0.0);
        context.move_to(x0 + 1.0, y);
        context.line_to(x0 + width, y);
        // A failed stroke only leaves the dots out.
        let _ = context.stroke();
        return;
    }
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

/// The search bar over the text (Ctrl+F in the editor): a relm4
/// component (docs/relm4-migration.md). Its model is the only state, only
/// its update changes it, and its widgets' handlers only send
/// [`EditorSearchInput`] messages. It never touches the text: what people
/// ask of it goes to the editor as [`EditorSearchOutput`]s, and the editor,
/// which owns the text view and the search of its buffer, selects the
/// matches, gives the text the keyboard back and tells the bar the count.
pub struct EditorSearch {
    /// "3 of 12", "No matches", or nothing.
    count: String,
}

/// What the editor's search bar is told.
#[derive(Debug)]
pub enum EditorSearchInput {
    /// Open the bar (Ctrl+F in the text) with this text to find (one line
    /// of the selection, read by the editor), or the last one.
    StartSearch(Option<String>),
    /// Open the bar on this text: [`Editor::find`] has already selected
    /// its first match from the cursor, at once, so whatever its caller
    /// does next acts on the match.
    Find(String),
    /// Close the bar; the editor hears [`EditorSearchOutput::Closed`],
    /// takes the highlights away and gives the text the keyboard.
    CloseSearch,
    /// What the editor counted: "3 of 12", "No matches", or nothing.
    ShowCount(String),
    /// People changed the text to find.
    Typed,
    /// People asked for the next match (Enter, Ctrl+G, the ↓ button).
    Next,
    /// People asked for the previous match (Shift+Enter, Ctrl+Shift+G,
    /// the ↑ button).
    Previous,
}

/// What the search bar tells the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorSearchOutput {
    /// The bar opened (or opened again) on this text to find: search the
    /// buffer showing for it.
    Opened(String),
    /// People changed the text to find to this.
    Typed(String),
    /// People asked for the next match (`forward`) or the previous one.
    Step {
        /// The next match, not the previous one.
        forward: bool,
    },
    /// The bar closed.
    Closed,
}

impl EditorSearch {
    /// `true` while the bar is open.
    fn is_open(root: &gtk::Revealer) -> bool {
        root.reveals_child()
    }

    /// Opens the bar with `text` to find, or the last one, and tells the
    /// editor what it searches for.
    fn open(
        text: Option<&str>,
        widgets: &EditorSearchWidgets,
        root: &gtk::Revealer,
        sender: &ComponentSender<Self>,
    ) {
        if let Some(text) = text {
            widgets.entry.set_text(text);
        }
        root.set_visible(true);
        root.set_reveal_child(true);
        // Nobody listening is not a mistake.
        let _ = sender.output(EditorSearchOutput::Opened(widgets.entry.text().into()));
        widgets.entry.grab_focus();
        widgets.entry.select_region(0, -1);
    }

    /// Closes the bar, telling the editor.
    fn close(&mut self, root: &gtk::Revealer, sender: &ComponentSender<Self>) {
        if !Self::is_open(root) {
            return;
        }
        root.set_reveal_child(false);
        root.set_visible(false);
        self.count.clear();
        let _ = sender.output(EditorSearchOutput::Closed);
    }
}

#[relm4::component(pub)]
impl Component for EditorSearch {
    type Init = ();
    type Input = EditorSearchInput;
    type Output = EditorSearchOutput;
    type CommandOutput = ();

    view! {
        gtk::Revealer {
            set_transition_type: gtk::RevealerTransitionType::SlideDown,
            set_halign: gtk::Align::End,
            set_valign: gtk::Align::Start,
            set_margin_end: 18,
            set_margin_top: 6,
            set_reveal_child: false,
            // Hidden while closed, so it takes no pointer events.
            set_visible: false,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 4,
                add_css_class: "card",

                #[name = "entry"]
                gtk::SearchEntry {
                    set_placeholder_text: Some(&tr("Find in this file")),
                    set_width_chars: 22,
                    set_margin_top: 6,
                    set_margin_bottom: 6,
                    set_margin_start: 6,
                    connect_search_changed[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::Typed);
                    },
                    connect_activate[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::Next);
                    },
                    connect_next_match[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::Next);
                    },
                    connect_previous_match[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::Previous);
                    },
                    connect_stop_search[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::CloseSearch);
                    },
                    // Shift+Enter goes back.
                    add_controller = gtk::EventControllerKey {
                        connect_key_pressed[sender] => move |_, key, _, modifiers| {
                            let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                            if shift && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter) {
                                sender.input_sender().emit(EditorSearchInput::Previous);
                                return glib::Propagation::Stop;
                            }
                            glib::Propagation::Proceed
                        },
                    },
                },

                gtk::Label {
                    set_css_classes: &["dim-label", "numeric"],
                    set_width_chars: 9,
                    set_xalign: 1.0,
                    #[watch]
                    set_label: &model.count,
                },

                gtk::Button {
                    set_icon_name: "go-up-symbolic",
                    set_tooltip_text: Some(&tr("Previous match (Shift+Enter)")),
                    // An icon button: its name for screen readers is the
                    // tooltip without the key.
                    update_property: &[gtk::accessible::Property::Label(&tr("Previous match"))],
                    set_css_classes: &["flat", "circular"],
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::Previous);
                    },
                },

                gtk::Button {
                    set_icon_name: "go-down-symbolic",
                    set_tooltip_text: Some(&tr("Next match (Enter)")),
                    // An icon button: its name for screen readers is the
                    // tooltip without the key.
                    update_property: &[gtk::accessible::Property::Label(&tr("Next match"))],
                    set_css_classes: &["flat", "circular"],
                    set_valign: gtk::Align::Center,
                    connect_clicked[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::Next);
                    },
                },

                gtk::Button {
                    set_icon_name: "window-close-symbolic",
                    set_tooltip_text: Some(&tr("Close the search (Escape)")),
                    // An icon button: its name for screen readers is the
                    // tooltip without the key.
                    update_property: &[gtk::accessible::Property::Label(&tr("Close the search"))],
                    set_css_classes: &["flat", "circular"],
                    set_valign: gtk::Align::Center,
                    set_margin_end: 6,
                    connect_clicked[sender] => move |_| {
                        sender.input_sender().emit(EditorSearchInput::CloseSearch);
                    },
                },
            },
        }
    }

    fn init((): (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = EditorSearch {
            count: String::new(),
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: EditorSearchInput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            EditorSearchInput::StartSearch(text) => {
                Self::open(text.as_deref(), widgets, root, &sender);
            }
            EditorSearchInput::Find(text) => Self::open(Some(&text), widgets, root, &sender),
            EditorSearchInput::CloseSearch => self.close(root, &sender),
            EditorSearchInput::ShowCount(count) => self.count = count,
            EditorSearchInput::Typed => {
                let _ = sender.output(EditorSearchOutput::Typed(widgets.entry.text().into()));
            }
            EditorSearchInput::Next => {
                let _ = sender.output(EditorSearchOutput::Step { forward: true });
            }
            EditorSearchInput::Previous => {
                let _ = sender.output(EditorSearchOutput::Step { forward: false });
            }
        }
        self.update_view(widgets, sender);
    }
}

/// How the search bar searches: round past the end, ignoring case.
fn search_settings() -> sourceview5::SearchSettings {
    sourceview5::SearchSettings::builder()
        .wrap_around(true)
        .case_sensitive(false)
        .build()
}

/// Selects the next (or previous) match `context` finds in the view's
/// text and scrolls to it; `again` starts from the selection's start, to
/// keep the match there.
fn select_match(
    view: &gtk::TextView,
    context: &sourceview5::SearchContext,
    forward: bool,
    again: bool,
) {
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
        return tr("No matches");
    }
    let position = buffer
        .selection_bounds()
        .map_or(0, |(start, end)| context.occurrence_position(&start, &end));
    if position >= 1 {
        fill_in(
            &tr("{position} of {total}"),
            &[("position", &position), ("total", &total)],
        )
    } else {
        fill_in(
            &ntr("{count} match", "{count} matches", total),
            &[("count", &total)],
        )
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
    /// The search bar: the editor owns its controller (dropping the
    /// editor shuts it down), its outputs forwarded as
    /// [`SearchEvent::Bar`] to `search_events`.
    search: Controller<EditorSearch>,
    /// How the search searches, and the text it finds.
    search_settings: sourceview5::SearchSettings,
    /// The search of the buffer showing, while the bar is open.
    search_context: Option<sourceview5::SearchContext>,
    /// The bar is open, or asked to open: set when the editor asks it to
    /// open, cleared when it asks it to close or hears that it closed
    /// ([`EditorSearchOutput::Closed`]), so it never lags behind what was
    /// asked of the bar.
    search_shown: bool,
    /// Where the search bar's outputs and the search's counts go: the
    /// owner forwards them into its own input and hands each back to
    /// [`Editor::search_event`].
    search_events: relm4::Sender<SearchEvent>,
    dark: bool,
    /// The lens rows, made as needed and kept (GTK can't take an overlay
    /// back out of a text view, so they are hidden when not used).
    lenses: Vec<LensSlot>,
    /// How many of the lens rows are in use.
    lenses_used: usize,
    /// The room above a line with a lens: its row's height and a gap
    /// either side, at least [`LENS_HEIGHT`] (see [`Editor::lens_room`]).
    lens_room: i32,
    /// Where the peek goes, and the line it sits under while it shows.
    peek_box: gtk::Box,
    peek_line: Option<i32>,
    /// The line and room the peek's tag was last applied for: applied
    /// again only when they change, since every change lays the text out
    /// again.
    peek_tagged: Option<(i32, i32)>,
    /// Where the hover card goes.
    card_box: gtk::Box,
    /// The frame callback of a go-to's scroll still waiting for the
    /// text's size: a newer go-to removes it.
    scroll: Option<gtk::TickCallbackId>,
    /// Where a lens's links send their clicks: the owner forwards them
    /// into its own input.
    lens: relm4::Sender<LensClick>,
    /// The rows of links above conflicts, made as needed and kept, like
    /// the lens rows.
    conflict_rows: Vec<ConflictRow>,
    /// How many of the conflict rows are in use.
    conflict_rows_used: usize,
}

/// The links above one conflict: "Keep yours · Keep theirs ·
/// Keep both".
struct ConflictRow {
    root: gtk::Box,
    /// The conflict's opening line, from 0.
    line: i32,
}

/// What the editor's search hears, for its owner to hand back to
/// [`Editor::search_event`] from its own update.
#[derive(Debug)]
pub enum SearchEvent {
    /// The search bar said this.
    Bar(EditorSearchOutput),
    /// The search of the buffer showing counted its matches again.
    Counted,
}

/// Which link of a lens was clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LensClick {
    /// "2 uses": list the declaration's uses.
    Uses(SymbolId),
    /// "show in 3D": select it in the 3D view.
    Show(SymbolId),
    /// A link above a conflict ("Keep yours", ...): settle it in the
    /// buffer showing ([`Editor::settle_conflict`]).
    Settle(ConflictSettle),
}

/// What a conflict's link asks: settle conflict number `index` (from 0)
/// by `resolution`, if it still reads `original`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictSettle {
    /// Which conflict of the text, from 0.
    pub index: usize,
    /// How to settle it.
    pub resolution: stave_lang::conflict::Resolution,
    /// The conflict's text when the link was made.
    pub original: String,
}

/// Which click a lens's link makes, given its declaration.
type LensLink = fn(SymbolId) -> LensClick;

/// Whether anything is open over the text that Escape closes (the hover
/// card, the peek, the search bar), read from the widgets alone, so a key
/// handler can answer GTK at once without asking the model.
#[derive(Clone)]
pub struct EscapeProbe {
    card: glib::WeakRef<gtk::Box>,
    peek: glib::WeakRef<gtk::Box>,
    search: glib::WeakRef<gtk::Revealer>,
}

impl EscapeProbe {
    /// `true` while the card, the peek or the search bar shows.
    pub fn anything_open(&self) -> bool {
        self.card.upgrade().is_some_and(|card| card.is_visible())
            || self.peek.upgrade().is_some_and(|peek| peek.is_visible())
            || self
                .search
                .upgrade()
                .is_some_and(|search| EditorSearch::is_open(&search))
    }
}

/// A link above a conflict that asks, on the lens channel, to settle it
/// ([`LensClick::Settle`]): its handlers only send.
fn conflict_link(
    lens: &relm4::Sender<LensClick>,
    label: &str,
    settle: ConflictSettle,
) -> gtk::Button {
    let button = super::style::as_link(
        gtk::Button::builder()
            .label(label)
            .css_classes(["flat", "stave-lens-link"])
            .build(),
    );
    // As on a lens: the text view under the row would take the press too
    // and start selecting, so the press is claimed and the link works on
    // release.
    let mouse = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_PRIMARY)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    mouse.connect_pressed(|gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    let send = {
        let lens = lens.clone();
        move || lens.emit(LensClick::Settle(settle.clone()))
    };
    let from_mouse = send.clone();
    mouse.connect_released(move |_, _, _, _| from_mouse());
    button.add_controller(mouse);
    button.connect_clicked(move |_| send());
    button
}

/// Settles conflict number `index` (from 0) of `buffer` by `resolution`
/// as one undoable change, if it still reads `original`. `false` when it
/// doesn't (the text changed since the link was made), and nothing is
/// changed.
fn settle_conflict(
    buffer: &gtk::TextBuffer,
    index: usize,
    resolution: &stave_lang::conflict::Resolution,
    original: &str,
) -> bool {
    let text = buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string();
    let still = stave_lang::conflict::find_conflicts(&text)
        .get(index)
        .is_some_and(|conflict| text.get(conflict.range.clone()) == Some(original));
    if !still {
        return false;
    }
    let Some(action) = stave_app::conflicts::actions(&text, index)
        .into_iter()
        .find(|action| action.resolution == *resolution)
    else {
        return false;
    };
    let (Ok(start), Ok(end)) = (
        u32::try_from(action.edit.range.start),
        u32::try_from(action.edit.range.end),
    ) else {
        return false;
    };
    let lines = LineStarts::new(&text);
    let mut from = iter_at(buffer, &lines, start);
    let mut to = iter_at(buffer, &lines, end);
    buffer.begin_user_action();
    buffer.delete(&mut from, &mut to);
    buffer.insert(&mut from, &action.edit.text);
    buffer.end_user_action();
    true
}

/// A new buffer for a document, sharing the editor's tags (`table`). Its
/// text can't be undone back to empty.
pub fn new_buffer(table: &gtk::TextTagTable, text: &str) -> gtk::TextBuffer {
    let buffer = sourceview5::Buffer::new(Some(table));
    buffer.set_highlight_syntax(false);
    buffer.set_highlight_matching_brackets(true);
    buffer.begin_irreversible_action();
    buffer.set_text(text);
    buffer.end_irreversible_action();
    buffer.place_cursor(&buffer.start_iter());
    buffer.upcast()
}

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
    /// The gestures that claim a press on each link (`uses`, `show`).
    presses: [gtk::GestureClick; 2],
    /// The handlers that call the links for the declaration the row shows
    /// now, made again each time it shows another (each captures the
    /// declaration it calls with).
    links: Vec<(glib::Object, glib::SignalHandlerId)>,
    /// The line it sits above, from 0.
    line: i32,
}

impl Editor {
    /// A conflict's link was clicked ([`LensClick::Settle`]): settles it
    /// in the buffer showing as one undoable change, if it still reads as
    /// it did when the link was made. `false` when it doesn't, and
    /// nothing changes.
    pub fn settle_conflict(&self, settle: &ConflictSettle) -> bool {
        settle_conflict(
            &self.view.buffer(),
            settle.index,
            &settle.resolution,
            &settle.original,
        )
    }

    /// An empty editor whose buffers share `table`; a lens's links send
    /// their clicks on `lens`, and the search sends what it hears on
    /// `search_events`.
    pub fn new(
        table: &gtk::TextTagTable,
        lens: relm4::Sender<LensClick>,
        search_events: relm4::Sender<SearchEvent>,
    ) -> Editor {
        let table = table.clone();
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
        let search = EditorSearch::builder()
            .launch(())
            .forward(&search_events, SearchEvent::Bar);
        let root = gtk::Overlay::builder().child(&scrolled).build();
        root.add_overlay(search.widget());
        let editor = Editor {
            root,
            view,
            code,
            scrolled,
            table,
            tags,
            marks,
            search,
            search_settings: search_settings(),
            search_context: None,
            search_shown: false,
            search_events,
            dark: false,
            lenses: Vec::new(),
            lenses_used: 0,
            lens_room: LENS_HEIGHT,
            peek_box,
            peek_line: None,
            peek_tagged: None,
            card_box,
            scroll: None,
            lens,
            conflict_rows: Vec::new(),
            conflict_rows_used: 0,
        };
        let mut editor = editor;
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
        new_buffer(&self.table, text)
    }

    /// Whether Escape has something to close, for a key handler.
    pub fn escape_probe(&self) -> EscapeProbe {
        EscapeProbe {
            card: self.card_box.downgrade(),
            peek: self.peek_box.downgrade(),
            search: self.search.widget().downgrade(),
        }
    }

    /// Shows a buffer; read-only documents can't be typed in.
    pub fn show_buffer(&mut self, buffer: &gtk::TextBuffer, editable: bool) {
        self.clear_overlays();
        if let Some(source) = buffer.downcast_ref::<sourceview5::Buffer>() {
            source.set_style_scheme(style_scheme(self.dark).as_ref());
        }
        self.view.set_buffer(Some(buffer));
        self.view.set_editable(editable);
        self.view.set_cursor_visible(true);
        // An open search goes on in the buffer now showing.
        if self.search_context.is_some() {
            self.attach_search();
            self.recount();
        }
        self.marks.queue_draw();
    }

    /// Follows the light or dark style.
    pub fn colour(&mut self, dark: bool) {
        self.dark = dark;
        self.tags.colour(dark);
        if let Ok(source) = self.view.buffer().downcast::<sourceview5::Buffer>() {
            source.set_style_scheme(style_scheme(dark).as_ref());
        }
        for widget in [
            self.code.upcast_ref::<gtk::Widget>(),
            self.marks.upcast_ref(),
        ] {
            if dark {
                widget.add_css_class(DARK_CLASS);
            } else {
                widget.remove_css_class(DARK_CLASS);
            }
            widget.queue_draw();
        }
    }

    /// Queues `input` for the search bar; once it has gone there is
    /// nothing to tell.
    fn tell_search(&self, input: EditorSearchInput) {
        let _ = self.search.sender().send(input);
    }

    /// Colours a buffer by its tokens, underlines the names that mean an
    /// item declared elsewhere, and tints its conflicts with their links
    /// above them ([`Editor::mark_conflicts`]).
    pub fn paint(
        &mut self,
        buffer: &gtk::TextBuffer,
        lines: &LineStarts,
        runs: &[(Run, TokenClass, TokenModifiers)],
    ) {
        let (start, end) = buffer.bounds();
        for tag in self.tags.syntax() {
            buffer.remove_tag(tag, &start, &end);
        }
        for (run, class, modifiers) in runs {
            let class_tag = self.tags.class(*class);
            let underline = theme::is_underlined(*modifiers).then_some(&self.tags.reference);
            if class_tag.is_none() && underline.is_none() {
                continue;
            }
            let from = iter_at(buffer, lines, run.start);
            let to = iter_at(buffer, lines, run.end);
            for tag in class_tag.into_iter().chain(underline) {
                buffer.apply_tag(tag, &from, &to);
            }
        }
        self.mark_conflicts(buffer);
    }

    /// Tints each conflict in the buffer (grammar, "Conflicts"): the
    /// marker lines grey, the first side green, the other sides blue and
    /// the base amber, and puts a row of links above its opening line,
    /// one for each way to settle it (`stave_app::conflicts::actions`).
    ///
    /// A link edits the buffer itself, as typing does, in one undoable
    /// change; the Files page then hears the buffer change as it hears
    /// typing. It reads the conflict from the buffer when clicked and
    /// settles nothing if the conflict is no longer the one it was made
    /// for.
    fn mark_conflicts(&mut self, buffer: &gtk::TextBuffer) {
        let (start, end) = buffer.bounds();
        for tag in self.tags.conflict() {
            buffer.remove_tag(tag, &start, &end);
        }
        let text = buffer.text(&start, &end, true).to_string();
        let found = stave_app::conflicts::conflicts(&text);
        let lines = LineStarts::new(&text);
        let range_tag = |tag: &gtk::TextTag, range: &std::ops::Range<usize>| {
            let (Ok(from), Ok(to)) = (u32::try_from(range.start), u32::try_from(range.end)) else {
                return;
            };
            buffer.apply_tag(
                tag,
                &iter_at(buffer, &lines, from),
                &iter_at(buffer, &lines, to),
            );
        };
        for file_conflict in &found {
            let conflict = &file_conflict.conflict;
            range_tag(&self.tags.conflict_marker, &conflict.range);
            for (index, side) in conflict.sides.iter().enumerate() {
                let tag = if index == 0 {
                    &self.tags.conflict_side
                } else {
                    &self.tags.conflict_other
                };
                range_tag(tag, &side.range);
            }
            for base in conflict.base.iter().chain(&conflict.later_bases) {
                range_tag(&self.tags.conflict_base, &base.range);
            }
        }
        while self.conflict_rows.len() < found.len() {
            let root = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .css_classes(["stave-lens"])
                .build();
            self.view.add_overlay(&root, 0, 0);
            self.conflict_rows.push(ConflictRow { root, line: 0 });
        }
        for (index, row) in self.conflict_rows.iter_mut().enumerate() {
            while let Some(child) = row.root.first_child() {
                row.root.remove(&child);
            }
            let Some(file_conflict) = found.get(index) else {
                row.root.set_visible(false);
                continue;
            };
            let original = text
                .get(file_conflict.conflict.range.clone())
                .unwrap_or_default()
                .to_owned();
            for (number, action) in file_conflict.actions.iter().enumerate() {
                if number > 0 {
                    let dot = gtk::Label::new(Some("·"));
                    dot.add_css_class("stave-lens-dot");
                    row.root.append(&dot);
                }
                row.root.append(&conflict_link(
                    &self.lens,
                    &action.label,
                    ConflictSettle {
                        index: file_conflict.index,
                        resolution: action.resolution.clone(),
                        original: original.clone(),
                    },
                ));
            }
            row.line = i32::try_from(file_conflict.lines.start().saturating_sub(1)).unwrap_or(0);
            row.root.set_visible(true);
            if let Some(from) = buffer.iter_at_line(row.line) {
                let mut to = from;
                to.forward_to_line_end();
                buffer.apply_tag(&self.tags.conflict_links, &from, &to);
            }
        }
        self.conflict_rows_used = found.len();
        self.place_overlays();
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
        self.view.has_focus() || super::keyboard_inside(self.search.widget().upcast_ref())
    }

    /// Opens the search bar (Ctrl+F in the text), when the main loop next
    /// runs, with the selection (one line of it) as the text to find.
    pub fn start_search(&mut self) {
        self.search_shown = true;
        let buffer = self.view.buffer();
        let selected = buffer
            .selection_bounds()
            .filter(|(start, end)| start.line() == end.line())
            .map(|(start, end)| buffer.text(&start, &end, false).to_string());
        self.tell_search(EditorSearchInput::StartSearch(selected));
    }

    /// Acts on what the search heard ([`SearchEvent`]), from the owner's
    /// update: searches the buffer showing for the bar's text, selects
    /// the next or previous match, and tells the bar the count.
    pub fn search_event(&mut self, event: SearchEvent) {
        match event {
            SearchEvent::Bar(EditorSearchOutput::Opened(text)) => {
                self.set_search_text(&text);
                self.attach_search();
                self.recount();
            }
            SearchEvent::Bar(EditorSearchOutput::Typed(text)) => {
                // Found from the start of the selection, so a longer text
                // keeps the match it had.
                self.set_search_text(&text);
                if !text.is_empty() {
                    self.step_search(true, true);
                }
                self.recount();
            }
            SearchEvent::Bar(EditorSearchOutput::Step { forward }) => {
                self.step_search(forward, false);
                self.recount();
            }
            SearchEvent::Bar(EditorSearchOutput::Closed) => {
                self.search_shown = false;
                self.search_settings.set_search_text(None);
                self.search_context = None;
                self.view.grab_focus();
            }
            SearchEvent::Counted => self.recount(),
        }
    }

    /// The text the search finds (none for an empty one).
    fn set_search_text(&self, text: &str) {
        self.search_settings
            .set_search_text(Some(text).filter(|text| !text.is_empty()));
    }

    /// Searches the buffer showing; its count comes back as
    /// [`SearchEvent::Counted`].
    fn attach_search(&mut self) {
        let buffer = self.view.buffer().downcast::<sourceview5::Buffer>().ok();
        self.search_context = buffer.map(|buffer| {
            let context = sourceview5::SearchContext::new(&buffer, Some(&self.search_settings));
            context.set_highlight(true);
            let events = self.search_events.clone();
            context.connect_occurrences_count_notify(move |_| {
                // Once the owner has gone there is nobody to tell.
                let _ = events.send(SearchEvent::Counted);
            });
            context
        });
    }

    /// Selects the next (or previous) match and scrolls to it; `again`
    /// starts from the selection's start, to keep the match there.
    fn step_search(&self, forward: bool, again: bool) {
        if let Some(context) = &self.search_context {
            select_match(&self.view, context, forward, again);
        }
    }

    /// Tells the bar the count: "3 of 12", "No matches", or nothing.
    fn recount(&self) {
        let text = self.search_settings.search_text().unwrap_or_default();
        let count = self
            .search_context
            .as_ref()
            .map(|context| count_text(context, &self.view.buffer(), &text))
            .unwrap_or_default();
        self.tell_search(EditorSearchInput::ShowCount(count));
    }

    /// Finds `text` from the cursor and selects it at once, as before, so
    /// what the caller does next (follow, uses, peek, hover at the cursor)
    /// acts on the match; the search bar opens on `text` when the main
    /// loop next runs. Selecting moves the cursor, so the text's own
    /// "mark-set" handlers run inside this call, as they always did; the
    /// search component is not running then, and anything they tell the
    /// bar is queued.
    pub fn find(&mut self, text: &str) {
        self.search_shown = true;
        if !text.is_empty()
            && let Ok(buffer) = self.view.buffer().downcast::<sourceview5::Buffer>()
        {
            let settings = search_settings();
            settings.set_search_text(Some(text));
            let context = sourceview5::SearchContext::new(&buffer, Some(&settings));
            context.set_highlight(false);
            select_match(&self.view, &context, true, true);
        }
        self.tell_search(EditorSearchInput::Find(text.to_owned()));
    }

    /// `true` while the search bar is open or asked to open, as the
    /// editor has asked of it and heard from it (never behind a queued
    /// open or close).
    pub fn has_search(&self) -> bool {
        self.search_shown
    }

    /// `true` while the bar shows, as of the last message it handled
    /// (tests).
    #[cfg(test)]
    fn bar_open(&self) -> bool {
        EditorSearch::is_open(self.search.widget())
    }

    /// Closes the search bar and its highlights, when the main loop next
    /// runs.
    pub fn close_search(&mut self) {
        self.search_shown = false;
        self.tell_search(EditorSearchInput::CloseSearch);
    }

    fn clear_overlays(&mut self) {
        for row in &self.conflict_rows {
            row.root.set_visible(false);
        }
        self.conflict_rows_used = 0;
        self.set_lenses(&[]);
        self.set_peek(None);
        self.hide_card();
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
                .label(tr("show in 3D"))
                .css_classes(["flat", "stave-lens-link"])
                .build(),
        );
        root.append(&uses);
        root.append(&dot);
        root.append(&show);
        // A button doesn't claim the press, so the text view under the
        // lens would take it too and start selecting text. This claims it
        // first (which also keeps the button's own click from firing);
        // the link is called on release ([`Editor::link_lens`]).
        let presses = [&uses, &show].map(|button| {
            let mouse = gtk::GestureClick::builder()
                .button(gtk::gdk::BUTTON_PRIMARY)
                .propagation_phase(gtk::PropagationPhase::Capture)
                .build();
            mouse.connect_pressed(|gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            });
            button.add_controller(mouse.clone());
            mouse
        });
        self.view.add_overlay(&root, 0, 0);
        LensSlot {
            root,
            uses,
            dot,
            show,
            presses,
            links: Vec::new(),
            line: 0,
        }
    }

    /// Makes a lens row's links send clicks for `symbol` (or nothing,
    /// `None`), replacing the handlers it had.
    fn link_lens(lens: &relm4::Sender<LensClick>, slot: &mut LensSlot, symbol: Option<SymbolId>) {
        for (object, handler) in slot.links.drain(..) {
            object.disconnect(handler);
        }
        let Some(symbol) = symbol else {
            return;
        };
        let links: [(&gtk::Button, &gtk::GestureClick, LensLink); 2] = [
            (&slot.uses, &slot.presses[0], LensClick::Uses),
            (&slot.show, &slot.presses[1], LensClick::Show),
        ];
        for (button, press, click) in links {
            // From the keyboard.
            let send = lens.clone();
            let keyboard = button.connect_clicked(move |_| send.emit(click(symbol)));
            slot.links.push((button.clone().upcast(), keyboard));
            // From the mouse, on release.
            let send = lens.clone();
            let mouse = press.connect_released(move |_, _, _, _| send.emit(click(symbol)));
            slot.links.push((press.clone().upcast(), mouse));
        }
    }

    /// Puts a lens above each of these lines, replacing the ones there.
    pub fn set_lenses(&mut self, lenses: &[LensLine]) {
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.lens, &start, &end);
        while self.lenses.len() < lenses.len() {
            let slot = self.lens_slot();
            self.lenses.push(slot);
        }
        for (index, slot) in self.lenses.iter_mut().enumerate() {
            match lenses.get(index) {
                Some(lens) => {
                    slot.uses.set_label(&lens.uses);
                    slot.uses.set_tooltip_text(Some(&fill_in(
                        &tr("List the uses of {name} (Shift+F12)"),
                        &[("name", &lens.name)],
                    )));
                    slot.show.set_tooltip_text(Some(&fill_in(
                        &tr("Select {name} in the 3D view"),
                        &[("name", &lens.name)],
                    )));
                    slot.dot.set_visible(lens.shows_in_3d);
                    slot.show.set_visible(lens.shows_in_3d);
                    Self::link_lens(&self.lens, slot, Some(lens.symbol));
                    slot.line = lens.line;
                    slot.root.set_visible(true);
                    if let Some(from) = buffer.iter_at_line(lens.line) {
                        let mut to = from;
                        to.forward_to_line_end();
                        buffer.apply_tag(&self.tags.lens, &from, &to);
                    }
                }
                None => {
                    slot.root.set_visible(false);
                    Self::link_lens(&self.lens, slot, None);
                }
            }
        }
        self.lenses_used = lenses.len();
        self.place_overlays();
    }

    /// Shows a peek under a line (from 0), or none.
    pub fn set_peek(&mut self, peek: Option<(gtk::Widget, i32)>) {
        while let Some(child) = self.peek_box.first_child() {
            self.peek_box.remove(&child);
        }
        let buffer = self.view.buffer();
        let (start, end) = buffer.bounds();
        buffer.remove_tag(&self.tags.peek, &start, &end);
        self.peek_tagged = None;
        match peek {
            Some((widget, line)) => {
                self.peek_box.append(&widget);
                self.peek_box.set_visible(true);
                self.peek_line = Some(line);
            }
            None => {
                self.peek_box.set_visible(false);
                self.peek_line = None;
            }
        }
        self.size_peek();
        self.place_overlays();
    }

    /// `true` while a peek is open.
    pub fn has_peek(&self) -> bool {
        self.peek_line.is_some()
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
    fn size_peek(&mut self) {
        let Some(line) = self.peek_line else {
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
        let lens_below = self
            .lenses
            .iter()
            .take(self.lenses_used)
            .any(|slot| slot.line == next);
        let mut room = LINE_SPACING.saturating_add(natural).saturating_add(10);
        if lens_below {
            room = room.saturating_add(self.lens_room);
        }
        if self.peek_tagged == Some((line, room)) {
            return;
        }
        self.peek_tagged = Some((line, room));
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

    /// Makes the room above each line with a lens as tall as a lens row
    /// is now, with a gap either side, and never less than
    /// [`LENS_HEIGHT`]: the row grows with the text (Large Text, a larger
    /// font) and must never reach down over the line or up over the line
    /// before. Changing the room lays the text out again, which settles
    /// the overlays once more.
    fn lens_room(&mut self) {
        let row = self
            .lenses
            .iter()
            .take(self.lenses_used)
            .map(|slot| slot.root.measure(gtk::Orientation::Vertical, -1).1)
            .max()
            .unwrap_or(0);
        let room = row
            .saturating_add(LENS_GAP.saturating_mul(2))
            .max(LENS_HEIGHT);
        if room != self.lens_room {
            self.lens_room = room;
            self.tags
                .lens
                .set_pixels_above_lines(LINE_SPACING.saturating_add(room));
        }
    }

    /// Moves the lenses and the peek to their lines, after the text moved.
    pub fn place_overlays(&mut self) {
        self.lens_room();
        self.size_peek();
        let buffer = self.view.buffer();
        for slot in self.lenses.iter().take(self.lenses_used) {
            if let Some(iter) = buffer.iter_at_line(slot.line) {
                // Just above the line's text, in the room above it.
                let text_top = self.view.iter_location(&iter).y();
                let y = text_top
                    .saturating_sub(LINE_SPACING)
                    .saturating_sub(self.lens_room)
                    .saturating_add(LENS_GAP);
                self.view.move_overlay(&slot.root, 2, y);
            }
        }
        for row in self.conflict_rows.iter().take(self.conflict_rows_used) {
            if let Some(iter) = buffer.iter_at_line(row.line) {
                let text_top = self.view.iter_location(&iter).y();
                let y = text_top
                    .saturating_sub(LINE_SPACING)
                    .saturating_sub(LENS_HEIGHT)
                    .saturating_add(2);
                self.view.move_overlay(&row.root, 2, y);
            }
        }
        if let Some(line) = self.peek_line
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
    pub fn go_to(&mut self, lines: &LineStarts, offset: u32) {
        let buffer = self.view.buffer();
        let iter = iter_at(&buffer, lines, offset);
        buffer.place_cursor(&iter);
        let mark = buffer.get_insert();
        // A newer go-to replaces one still waiting.
        if let Some(older) = self.scroll.take() {
            older.remove();
        }
        let scrolled = self.scrolled.downgrade();
        // The frame it was asked at, to give up after a while: the view's
        // frame counter, once it has a frame clock (until then no tick
        // comes).
        let asked_at = self.view.frame_clock().map(|clock| clock.frame_counter());
        // Ticks come once a frame, and not while the view is off screen.
        let callback = self.view.add_tick_callback(move |view, clock| {
            let Some(scrolled) = scrolled.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if mark.buffer().as_ref() != Some(&view.buffer()) {
                return glib::ControlFlow::Break;
            }
            let frames = asked_at.map_or(1, |asked_at| {
                clock.frame_counter().saturating_sub(asked_at).max(1)
            });
            let vertical = scrolled.vadjustment();
            let page = Page {
                value: vertical.value(),
                size: vertical.page_size(),
                lower: vertical.lower(),
                upper: vertical.upper(),
            };
            if !page.is_laid_out() || view.height() <= 1 {
                return if frames < i64::from(SCROLL_WAIT_FRAMES) {
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
        self.scroll = Some(callback);
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
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, recorder, settle, settle_until};

    /// What an editor's search heard, kept until handed back.
    type Heard = Rc<RefCell<Vec<SearchEvent>>>;

    /// An editor whose search events are kept in `Heard`, as the Files
    /// component's input would queue them.
    fn editor_hearing() -> (Editor, Heard) {
        let (search_events, heard) = recorder();
        let editor = Editor::new(&gtk::TextTagTable::new(), relm4::channel().0, search_events);
        (editor, heard)
    }

    /// Runs the main loop and hands the editor what its search heard,
    /// until nothing more comes, as the Files component's update does.
    fn pump(editor: &mut Editor, heard: &Heard) {
        loop {
            settle();
            let events: Vec<SearchEvent> = heard.borrow_mut().drain(..).collect();
            if events.is_empty() {
                return;
            }
            for event in events {
                editor.search_event(event);
            }
        }
    }

    #[test]
    fn a_conflict_is_tinted_and_its_links_settle_it() {
        on_gtk_thread(conflict_links_settle);
    }

    /// The links of a conflict row, in order.
    fn links(row: &ConflictRow) -> Vec<gtk::Button> {
        let mut found = Vec::new();
        let mut child = row.root.first_child();
        while let Some(widget) = child {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                found.push(button.clone());
            }
            child = widget.next_sibling();
        }
        found
    }

    /// The link of a conflict row whose text is `label`.
    fn link(row: &ConflictRow, label: &str) -> gtk::Button {
        links(row)
            .into_iter()
            .find(|button| button.label().as_deref() == Some(label))
            .unwrap_or_else(|| panic!("no link {label}"))
    }

    fn whole_text(buffer: &gtk::TextBuffer) -> String {
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), true)
            .to_string()
    }

    /// A conflict's sides are tinted, a row of links sits above its
    /// opening line, and a link asks (on the lens channel) to replace the
    /// conflict in the buffer, which the editor does as one undoable
    /// change when its owner hands the click back. A link left from
    /// before the conflict changed settles nothing.
    fn conflict_links_settle() {
        let (lens, clicks) = recorder::<LensClick>();
        let mut editor = Editor::new(&gtk::TextTagTable::new(), lens, relm4::channel().0);
        // What the Files component does with each click it hears.
        let hand_back = |editor: &Editor| {
            let mut settled = Vec::new();
            settle();
            let heard: Vec<LensClick> = clicks.borrow_mut().drain(..).collect();
            for click in heard {
                match click {
                    LensClick::Settle(settle) => settled.push(editor.settle_conflict(&settle)),
                    LensClick::Uses(_) | LensClick::Show(_) => {}
                }
            }
            settled
        };
        let text = "stave 1\n\n<<<<<<< conflict 1 of 1\n+++++++ yours\nlet a = 1\n------- before\n+++++++ theirs\nlet b = 2\n>>>>>>> conflict 1 of 1 ends\n";
        let buffer = editor.new_buffer(text);
        editor.show_buffer(&buffer, true);
        editor.paint(&buffer, &LineStarts::new(text), &[]);

        assert_eq!(editor.conflict_rows_used, 1);
        let row = editor.conflict_rows.first().expect("a row of links");
        assert!(row.root.is_visible());
        assert_eq!(row.line, 2, "above the opening marker line");
        let labels: Vec<String> = links(row)
            .iter()
            .map(|button| {
                button
                    .label()
                    .map(|label| label.to_string())
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(labels, ["Keep yours", "Keep theirs", "Keep both"]);
        let tagged = |line: i32, name: &str| {
            buffer.iter_at_line(line).is_some_and(|iter| {
                iter.tags()
                    .iter()
                    .any(|tag| tag.name().as_deref() == Some(name))
            })
        };
        assert!(tagged(2, "conflict-marker"));
        assert!(tagged(4, "conflict-side"));
        assert!(tagged(7, "conflict-other"));
        assert!(!tagged(0, "conflict-marker"));

        // A link from before the conflict changed settles nothing.
        let stale = link(row, "Keep yours");
        if let Some(mut at) = buffer.iter_at_line(4) {
            buffer.insert(&mut at, "let c = 3\n");
        }
        let changed = whole_text(&buffer);
        stale.emit_clicked();
        assert_eq!(whole_text(&buffer), changed, "the link only asks");
        assert_eq!(hand_back(&editor), [false]);
        assert_eq!(whole_text(&buffer), changed);

        // Painted again, the new row's link settles the conflict, as one
        // change that undo takes back.
        editor.paint(&buffer, &LineStarts::new(&changed), &[]);
        let row = editor.conflict_rows.first().expect("a row of links");
        link(row, "Keep both").emit_clicked();
        assert_eq!(whole_text(&buffer), changed, "the link only asks");
        assert_eq!(hand_back(&editor), [true]);
        assert_eq!(
            whole_text(&buffer),
            "stave 1\n\nlet c = 3\nlet a = 1\nlet b = 2\n"
        );
        let settled = whole_text(&buffer);
        editor.paint(&buffer, &LineStarts::new(&settled), &[]);
        assert_eq!(editor.conflict_rows_used, 0);
        assert!(!editor.conflict_rows.iter().any(|row| row.root.is_visible()));
        if let Some(source) = buffer.downcast_ref::<sourceview5::Buffer>() {
            source.undo();
        }
        assert_eq!(whole_text(&buffer), changed);
        settle();
    }

    #[test]
    fn a_dropped_editor_frees_its_search_bar() {
        on_gtk_thread(search_bar_is_freed);
    }

    /// Dropping the editor drops the search bar's controller, which shuts
    /// the component down when the main loop next runs: the bar, the
    /// model (with the search and the input channel the runtime held) and
    /// the editor's widgets are all freed.
    fn search_bar_is_freed() {
        let (mut editor, heard) = editor_hearing();
        let buffer = editor.new_buffer("door\nshelf\n");
        editor.show_buffer(&buffer, true);
        editor.find("door");
        pump(&mut editor, &heard);
        assert!(editor.bar_open(), "the bar is open");
        assert!(editor.search_context.is_some(), "and the buffer searched");
        let context = editor
            .search_context
            .as_ref()
            .map(glib::object::ObjectExt::downgrade);
        let bar = editor.search.widget().downgrade();
        let root = editor.root.downgrade();
        let view = editor.view.downgrade();
        let input = editor.search.sender().clone();
        drop(editor);
        settle();
        assert!(bar.upgrade().is_none(), "the search bar is freed");
        assert!(root.upgrade().is_none(), "the editor is freed");
        assert!(view.upgrade().is_none(), "the text view is freed");
        assert!(
            input.send(EditorSearchInput::CloseSearch).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(
            context.and_then(|context| context.upgrade()).is_none(),
            "the search of the buffer is freed"
        );
    }

    #[test]
    fn find_selects_the_match_before_it_returns() {
        on_gtk_thread(find_selects_at_once);
    }

    /// The Files page's `--search` wish finds the text and then, in the
    /// same call, follows, lists the uses of, peeks at or hovers over what
    /// is at the cursor: the match must be selected before `find` returns.
    /// The bar opening when the main loop runs leaves the match selected.
    fn find_selects_at_once() {
        let (mut editor, heard) = editor_hearing();
        let text = "shelf\npart side\nshelf\n";
        let lines = LineStarts::new(text);
        let buffer = editor.new_buffer(text);
        editor.show_buffer(&buffer, true);
        if let Some(line) = buffer.iter_at_line(1) {
            buffer.place_cursor(&line);
        }
        let second = text
            .rfind("shelf")
            .and_then(|at| u32::try_from(at).ok())
            .unwrap_or_default();

        editor.find("shelf");
        assert_eq!(
            editor.cursor(&lines),
            second,
            "the cursor is on the next shelf as soon as find returns"
        );
        assert!(editor.has_search(), "asked for at once");
        assert!(!editor.bar_open(), "the bar opens when the loop runs");

        pump(&mut editor, &heard);
        assert!(editor.bar_open(), "the bar is open");
        assert_eq!(
            editor.cursor(&lines),
            second,
            "opening the bar did not move on to another match"
        );
        let selected = buffer
            .selection_bounds()
            .map(|(start, end)| buffer.text(&start, &end, false).to_string());
        assert_eq!(selected.as_deref(), Some("shelf"));
    }

    #[test]
    fn a_lens_row_calls_for_the_declaration_it_shows_now() {
        on_gtk_thread(lens_rows_call_for_what_they_show);
    }

    /// A lens row is kept and shown again for another declaration: its
    /// links call for the one it shows at the click (each handler made
    /// again with it), and a hidden row calls for nothing.
    fn lens_rows_call_for_what_they_show() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/wall_shelf")
            .canonicalize()
            .unwrap_or_default();
        let analysis = stave_ide::Analysis::load(&folder, &stave_ide::Overlay::new())
            .unwrap_or_else(|_| panic!("the wall shelf loads"));
        let symbols: Vec<SymbolId> = analysis
            .project_files()
            .into_iter()
            .flat_map(|(file, _)| analysis.lenses(file))
            .map(|lens| lens.symbol)
            .collect();
        let [first, second, ..] = symbols.as_slice() else {
            panic!("the wall shelf declares two things");
        };
        let (lens, heard) = recorder();
        let mut editor = Editor::new(&gtk::TextTagTable::new(), lens, relm4::channel().0);
        let buffer = editor.new_buffer("a\nb\n");
        editor.show_buffer(&buffer, true);
        let lens = |symbol: SymbolId| LensLine {
            line: 0,
            uses: "1 use".to_owned(),
            shows_in_3d: true,
            symbol,
            name: "a".to_owned(),
        };
        editor.set_lenses(&[lens(*first)]);
        let (uses, show) = {
            let slot = editor.lenses.first().expect("a lens row");
            (slot.uses.clone(), slot.show.clone())
        };
        uses.emit_clicked();
        editor.set_lenses(&[lens(*second)]);
        uses.emit_clicked();
        show.emit_clicked();
        editor.set_lenses(&[]);
        uses.emit_clicked();
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                LensClick::Uses(*first),
                LensClick::Uses(*second),
                LensClick::Show(*second),
            ]
        );
        assert_eq!(editor.lenses.len(), 1, "the row was kept");
    }

    #[test]
    fn a_lens_sits_in_the_room_above_its_line_and_never_over_code() {
        on_gtk_thread(lenses_never_cover_code);
    }

    /// Each lens row sits wholly in the room above its line, below the
    /// line before: as laid out, after the text is scrolled, and after its
    /// font grows (Large Text).
    fn lenses_never_cover_code() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/wall_shelf")
            .canonicalize()
            .unwrap_or_default();
        let analysis = stave_ide::Analysis::load(&folder, &stave_ide::Overlay::new())
            .unwrap_or_else(|_| panic!("the wall shelf loads"));
        let Some(symbol) = analysis
            .project_files()
            .into_iter()
            .flat_map(|(file, _)| analysis.lenses(file))
            .map(|lens| lens.symbol)
            .next()
        else {
            panic!("the wall shelf declares something");
        };
        let (mut editor, _heard) = editor_hearing();
        let text: String = (0..60)
            .map(|line| format!("part p{line} : oak_19 {{\n"))
            .collect();
        let buffer = editor.new_buffer(&text);
        editor.show_buffer(&buffer, true);
        let window = gtk::Window::builder()
            .default_width(700)
            .default_height(500)
            .child(&editor.root)
            .build();
        // The window's own style: a lens row's links are as small as Stave
        // makes them.
        crate::ui::style::install(&gtk::prelude::WidgetExt::display(&window));
        window.present();
        let lines = [1, 13, 24, 26, 40];
        editor.set_lenses(
            &lines
                .iter()
                .map(|&line| LensLine {
                    line,
                    uses: "14 uses".to_owned(),
                    shows_in_3d: true,
                    symbol,
                    name: "p".to_owned(),
                })
                .collect::<Vec<_>>(),
        );
        let check = |editor: &Editor, when: &str| -> Result<(), String> {
            let view = &editor.view;
            let buffer = view.buffer();
            for slot in editor.lenses.iter().take(editor.lenses_used) {
                let Some(iter) = buffer.iter_at_line(slot.line) else {
                    panic!("line {} is there", slot.line);
                };
                let place = view.iter_location(&iter);
                let (_, text_top) =
                    view.buffer_to_window_coords(gtk::TextWindowType::Widget, 0, place.y());
                let Some(bounds) = slot.root.compute_bounds(view) else {
                    continue;
                };
                let top = f64::from(bounds.y());
                let bottom = top + f64::from(bounds.height());
                if bottom > f64::from(text_top) {
                    return Err(format!(
                        "{when}: the lens of line {} ends at {bottom}, below its text at {text_top}",
                        slot.line
                    ));
                }
                if slot.line > 0 {
                    let Some(before) = buffer.iter_at_line(slot.line - 1) else {
                        continue;
                    };
                    let place = view.iter_location(&before);
                    let (_, before_bottom) = view.buffer_to_window_coords(
                        gtk::TextWindowType::Widget,
                        0,
                        place.y() + place.height(),
                    );
                    if top < f64::from(before_bottom) {
                        return Err(format!(
                            "{when}: the lens of line {} starts at {top}, over the line before, which ends at {before_bottom}",
                            slot.line
                        ));
                    }
                }
            }
            Ok(())
        };
        // Lays the window out until every lens is in its room, or a few
        // seconds pass: the overlays follow the text as the files page's
        // settling has them, and changing a lens's room lays the text out
        // again. GTK lays a window out on its frame clock, which a private
        // Broadway display with no browser never ticks, so each round also
        // lays the editor out itself, at the size it has, as a frame would.
        let laid_out = |editor: &mut Editor, when: &str| {
            let mut last = Ok(());
            let settled = settle_until(std::time::Duration::from_secs(5), || {
                editor.place_overlays();
                let root = &editor.root;
                let (width, height) = (root.width(), root.height());
                let _ = root.measure(gtk::Orientation::Horizontal, -1);
                let _ = root.measure(gtk::Orientation::Vertical, width);
                root.allocate(width, height, -1, None);
                last = check(editor, when);
                last.is_ok()
            });
            if let Err(problem) = last {
                panic!("{problem}");
            }
            assert!(settled, "{when}: laid out");
        };
        laid_out(&mut editor, "laid out");
        editor.vadjustment().set_value(200.0);
        laid_out(&mut editor, "scrolled");
        let provider = gtk::CssProvider::new();
        provider.load_from_string("textview, .stave-lens { font-size: 20px; }");
        gtk::style_context_add_provider_for_display(
            &gtk::prelude::WidgetExt::display(&window),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
        laid_out(&mut editor, "with larger text");
        gtk::style_context_remove_provider_for_display(
            &gtk::prelude::WidgetExt::display(&window),
            &provider,
        );
        window.destroy();
    }

    #[test]
    fn closing_the_search_from_inside_its_own_step_is_queued_not_re_entered() {
        on_gtk_thread(search_is_queued);
    }

    /// When the old search bar selected a match, GTK's "mark-set" ran the
    /// Files page's handlers right then, inside the step: one that closed
    /// the search dropped the search the step was still using and moved
    /// the keyboard mid-step. Now the bar only asks for the step
    /// ([`EditorSearchOutput::Step`]), the editor makes it from its owner's
    /// update, and a close asked for from inside the step is queued until
    /// the step is done; the editor hears the bar close after it.
    fn search_is_queued() {
        let (mut editor, heard) = editor_hearing();
        let buffer = editor.new_buffer("door\nshelf\ndoor\n");
        editor.show_buffer(&buffer, true);
        editor.find("door");
        assert!(!editor.bar_open(), "opening the bar is only queued");
        pump(&mut editor, &heard);
        assert!(editor.bar_open(), "the bar is open");
        let selected = |buffer: &gtk::TextBuffer| {
            buffer
                .selection_bounds()
                .map(|(start, end)| (start.line(), buffer.text(&start, &end, false).to_string()))
        };
        assert_eq!(selected(&buffer), Some((0, "door".to_owned())));

        // Whether the bar was still open each time the cursor moved while
        // the editor stepped, after closing it from there.
        let open: Rc<RefCell<Vec<bool>>> = Rc::default();
        let log = Rc::clone(&open);
        let bar = editor.search.widget().downgrade();
        let input = editor.search.sender().clone();
        let handler = buffer.connect_mark_set(move |buffer, _, mark| {
            if *mark == buffer.get_insert() {
                let _ = input.send(EditorSearchInput::CloseSearch);
                log.borrow_mut()
                    .push(bar.upgrade().is_some_and(|bar| EditorSearch::is_open(&bar)));
            }
        });
        // Enter in the bar: the next match.
        editor.tell_search(EditorSearchInput::Next);
        assert!(open.borrow().is_empty(), "the step is only queued");
        pump(&mut editor, &heard);
        assert!(!open.borrow().is_empty(), "the step moved the cursor");
        assert!(
            open.borrow().iter().all(|open| *open),
            "the close waited for the step"
        );
        assert!(!editor.bar_open(), "and then closed the bar");
        assert!(!editor.has_search(), "which the editor heard");
        assert!(
            editor.search_context.is_none(),
            "the editor heard it close and stopped searching"
        );
        assert_eq!(
            selected(&buffer),
            Some((2, "door".to_owned())),
            "the step found the next door"
        );
        buffer.disconnect(handler);
    }

    #[test]
    fn the_bar_asks_and_the_editor_searches_and_counts() {
        on_gtk_thread(bar_asks_editor_counts);
    }

    /// The bar never touches the text: what people type and step reaches
    /// the editor as outputs, the editor selects the match, and the count
    /// it works out comes back to the bar as a message.
    fn bar_asks_editor_counts() {
        let (mut editor, heard) = editor_hearing();
        let buffer = editor.new_buffer("door\nshelf\ndoor\n");
        editor.show_buffer(&buffer, true);
        if let Some((start, end)) = buffer.iter_at_line(1).zip(buffer.iter_at_line_offset(1, 5)) {
            buffer.select_range(&start, &end);
        }
        editor.start_search();
        pump(&mut editor, &heard);
        assert!(
            editor.bar_open() && editor.has_search(),
            "Ctrl+F opened the bar"
        );
        assert_eq!(
            editor.search_settings.search_text().as_deref(),
            Some("shelf"),
            "on the selection, which the editor read"
        );
        assert_eq!(editor.search.model().count, "1 of 1");
        editor.tell_search(EditorSearchInput::Next);
        let _ = editor.search.sender().send(EditorSearchInput::Typed);
        pump(&mut editor, &heard);
        assert!(
            heard.borrow().is_empty(),
            "everything the bar said was handed back"
        );
        assert_eq!(editor.search.model().count, "1 of 1");
        editor.close_search();
        pump(&mut editor, &heard);
        assert!(!editor.bar_open() && !editor.has_search(), "closed");
        assert!(editor.search_context.is_none(), "no longer searching");
        assert_eq!(editor.search_settings.search_text(), None);
    }

    #[test]
    fn an_errors_gutter_bar_is_whole_and_a_warnings_dashed() {
        assert_eq!(bar_pieces(Severity::Error, 10, 13), [(10, 13)]);
        assert_eq!(
            bar_pieces(Severity::Warning, 10, 13),
            [(10, 3), (15, 3), (20, 3)]
        );
        assert_eq!(bar_pieces(Severity::Warning, 0, 4), [(0, 3)]);
        assert_eq!(bar_pieces(Severity::Warning, 0, 6), [(0, 3), (5, 1)]);
    }

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
