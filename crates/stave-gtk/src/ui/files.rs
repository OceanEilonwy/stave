//! The Code panel: the open files as tabs (one each with a dot for its
//! worst problem and a yellow ● while unsaved, shown in the strip of the
//! frame holding the panel, beside the Model: [`FilesPage::page_tabs`]),
//! then breadcrumbs, the editor, the uses panel and a footer; "No file
//! open" while none is. The panel is docked
//! wherever people put it, so files open there. Files are opened from the
//! Files panel, with the finder (Ctrl+K) or by following a link.
//!
//! Everything it knows about the text comes from `stave-ide`: the open
//! buffers ([`Buffers`]), and an [`Analysis`] built again on a worker
//! thread a moment after each change, from the buffers as typed. The
//! window decides what a name means in the design and owns the history:
//! this page tells it where the cursor is ([`FilesPage::connect_link`]),
//! where to go ([`FilesPage::connect_go`]) and when typing has paused
//! ([`FilesPage::connect_check`]), and it calls back with a place to show.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use stave_app::check::Checked;
use stave_app::session::{LineNumber, SourceLocation};
use stave_ide::{
    Analysis, Buffers, Document, FilePosition, Location, Overlay, SymbolId, TokenClass,
};
use stave_lang::Severity;
use stave_lang::ast::Span;

use super::editor::{Editor, LensLine, LineStarts, Run};

/// How long typing must pause before the names are worked out again.
const ANALYSE_DELAY: Duration = Duration::from_millis(150);
/// How long typing must pause before the design is checked again.
const CHECK_DELAY: Duration = Duration::from_millis(700);
/// How long the cursor must rest on a name before the 3D view follows.
const LINK_DELAY: Duration = Duration::from_millis(120);
/// How long the pointer must rest on a word before its card shows.
const HOVER_DELAY: Duration = Duration::from_millis(450);

/// What the screenshot asks the editor to show, once the names are in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wish {
    /// Put the cursor on the first occurrence of this word on the line.
    pub cursor: Option<String>,
    /// Peek at the definition of the word under the cursor.
    pub peek: bool,
    /// Show the card for the word under the cursor.
    pub hover: bool,
    /// List the uses of the word under the cursor.
    pub uses: bool,
    /// Type this text at the cursor, as if typed.
    pub typing: Option<String>,
    /// Follow the name under the cursor to its declaration (F12).
    pub follow: bool,
    /// Undo once, after any typing.
    pub undo: bool,
    /// Find this text in the file (Ctrl+F), from the cursor.
    pub search: Option<String>,
}

type LinkCallback = Rc<dyn Fn(CursorName)>;

/// What the cursor rests on, for the 3D view to follow.
#[derive(Clone)]
pub enum CursorName {
    /// A name that means this symbol.
    Name(Arc<Analysis>, SymbolId),
    /// A name that means nothing (a typo): the link lets go.
    Unresolved,
    /// Anything else (a keyword, a field name, a number, a space, or text
    /// not worked out yet): the link stays as it is.
    Other,
}
type GoCallback = Rc<dyn Fn(Option<SourceLocation>, SourceLocation)>;
type SymbolCallback = Rc<dyn Fn(Arc<Analysis>, SymbolId)>;

#[derive(Default)]
struct Callbacks {
    on_link: Option<LinkCallback>,
    on_go: Option<GoCallback>,
    on_check: Option<Rc<dyn Fn()>>,
    on_show_in_3d: Option<SymbolCallback>,
    on_open: Option<Rc<dyn Fn()>>,
    on_notice: Option<Rc<dyn Fn(String)>>,
}

/// One open document.
struct Tab {
    document: Document,
    buffer: gtk::TextBuffer,
    lines: LineStarts,
    button: gtk::Button,
    label: gtk::Label,
    /// The dot for the file's worst problem.
    dot: gtk::Box,
}

/// A problem the last check found, where it is.
#[derive(Debug, Clone)]
struct Problem {
    document: Document,
    span: Span,
    severity: Severity,
    message: String,
    help: Option<String>,
}

#[derive(Default)]
struct State {
    folder: PathBuf,
    buffers: Buffers,
    analysis: Option<Arc<Analysis>>,
    generation: u64,
    analysing: bool,
    analyse_again: bool,
    tabs: Vec<Tab>,
    active: Option<usize>,
    problems: Vec<Problem>,
    /// The text the last check read for each document it has a problem
    /// in: its problems are underlined only while the document still
    /// reads so (their spans are offsets into that text).
    checked_texts: std::collections::BTreeMap<Document, String>,
    /// Changes made by the page itself: don't report them.
    quiet: bool,
    /// The exact place to put the cursor when the next place shown is
    /// this location.
    pending: Option<(SourceLocation, u32)>,
    analyse_timer: Option<glib::SourceId>,
    check_timer: Option<glib::SourceId>,
    link_timer: Option<glib::SourceId>,
    hover_timer: Option<glib::SourceId>,
    /// The text the hover card is about, while it shows: the pointer
    /// leaving it hides the card at once.
    hover_span: Option<Span>,
    /// The worst problem in each file, by its source path.
    marks: stave_app::problems::Marks,
    wish: Option<Wish>,
    /// The symbol whose uses the panel lists.
    listed: Option<SymbolId>,
    /// Frames left in which the overlays follow the text's layout.
    settling: u32,
    /// The text changed: tell the window what the cursor rests on once
    /// the names are worked out again.
    relink: bool,
}

/// The Code panel: the open files as tabs, and the editor.
pub struct FilesPage {
    /// The panel's content.
    pub root: gtk::Box,
    tab_list: gtk::Box,
    /// What shows while no file is open.
    placeholder: gtk::Widget,
    /// The open file's text and what goes with it.
    body: gtk::Box,
    crumbs: gtk::Box,
    editor: Editor,
    uses: gtk::Revealer,
    uses_title: gtk::Label,
    uses_list: gtk::ListBox,
    line: gtk::Label,
    state: RefCell<State>,
    callbacks: RefCell<Callbacks>,
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(classes.to_vec())
        .build()
}

/// `true` if two texts are the analysis's text and the buffer's.
fn same_text(analysis: &Analysis, document: &Document, text: &str) -> Option<stave_lang::FileId> {
    let file = analysis.file(document)?;
    (analysis.text(file)? == text).then_some(file)
}

fn escape(text: &str) -> String {
    glib::markup_escape_text(text).to_string()
}

impl FilesPage {
    /// An empty rail and editor.
    pub fn new() -> Rc<FilesPage> {
        // The open files, one tab each, across the top.
        let tab_list = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .css_classes(["stave-code-tabs"])
            .build();
        let head = gtk::ScrolledWindow::builder()
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hscrollbar_policy(gtk::PolicyType::External)
            .child(&tab_list)
            .css_classes(["stave-code-head"])
            .build();

        let crumbs = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["stave-crumbs"])
            .build();

        let editor = Editor::new();

        // The uses panel.
        let uses_title = label("", &["stave-uses-title"]);
        uses_title.set_hexpand(true);
        let uses_close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Close the list of uses (Escape)")
            .css_classes(["flat", "circular"])
            .build();
        let uses_head = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-uses-head"])
            .build();
        uses_head.append(&uses_title);
        uses_head.append(&uses_close);
        let uses_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["stave-uses-list"])
            .build();
        let uses_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-uses"])
            .build();
        uses_box.append(&uses_head);
        uses_box.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(220)
                .child(&uses_list)
                .build(),
        );
        let uses = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideUp)
            .child(&uses_box)
            .reveal_child(false)
            .build();

        // The footer.
        let hint = label(
            "Ctrl+click follows a name · Alt+F12 peeks · Alt+← goes back",
            &["stave-footer-hint"],
        );
        hint.set_hexpand(true);
        hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
        hint.set_max_width_chars(10);
        let line = label("", &["stave-footer-line"]);
        let footer = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .css_classes(["stave-editor-footer"])
            .build();
        footer.append(&hint);
        footer.append(&line);

        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .vexpand(true)
            .visible(false)
            .build();
        body.append(&crumbs);
        body.append(&editor.root);
        body.append(&uses);
        body.append(&footer);
        // With no file open the panel stays where it is and says how to
        // open one.
        let placeholder = adw::StatusPage::builder()
            .icon_name("text-x-generic-symbolic")
            .title("No file open")
            .description("Open a file from Files, or find one with Ctrl+K. Files open in this panel, wherever you dock it.")
            .vexpand(true)
            .css_classes(["compact", "stave-code-placeholder"])
            .build()
            .upcast::<gtk::Widget>();

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-editor-column"])
            .build();
        // The open files show as tabs in the strip of the frame holding
        // this panel (`page_tabs`), beside the Model; this row stays out
        // of sight.
        head.set_visible(false);
        column.append(&head);
        column.append(&placeholder);
        column.append(&body);

        let page = Rc::new(FilesPage {
            root: column,
            tab_list,
            placeholder,
            body,
            crumbs,
            editor,
            uses,
            uses_title,
            uses_list,
            line,
            state: RefCell::new(State::default()),
            callbacks: RefCell::new(Callbacks::default()),
        });
        page.connect(&uses_close);
        page
    }

    // ---- what the window hears ----

    /// The cursor rests on a name, a broken name, or something else.
    pub fn connect_link(&self, callback: impl Fn(CursorName) + 'static) {
        self.callbacks.borrow_mut().on_link = Some(Rc::new(callback));
    }

    /// Go to a place, remembering where the cursor was (a history entry).
    pub fn connect_go(&self, callback: impl Fn(Option<SourceLocation>, SourceLocation) + 'static) {
        self.callbacks.borrow_mut().on_go = Some(Rc::new(callback));
    }

    /// Typing paused: check the design again, reading the open buffers.
    pub fn connect_check(&self, callback: impl Fn() + 'static) {
        self.callbacks.borrow_mut().on_check = Some(Rc::new(callback));
    }

    /// A lens's "show in 3D" was clicked.
    pub fn connect_show_in_3d(&self, callback: impl Fn(Arc<Analysis>, SymbolId) + 'static) {
        self.callbacks.borrow_mut().on_show_in_3d = Some(Rc::new(callback));
    }

    /// A tab on the rail was chosen: open the editor.
    pub fn connect_open(&self, callback: impl Fn() + 'static) {
        self.callbacks.borrow_mut().on_open = Some(Rc::new(callback));
    }

    /// Something to tell people (a failed save).
    pub fn connect_notice(&self, callback: impl Fn(String) + 'static) {
        self.callbacks.borrow_mut().on_notice = Some(Rc::new(callback));
    }

    fn notice(&self, text: String) {
        let callback = self.callbacks.borrow().on_notice.clone();
        if let Some(callback) = callback {
            callback(text);
        }
    }

    // ---- what the window asks ----

    /// Puts the keyboard in the text.
    pub fn focus_text(&self) {
        self.editor.view.grab_focus();
    }

    /// Opens a project folder: closes every tab (the window has asked
    /// about unsaved changes first) and lists the new folder's files.
    pub fn set_folder(self: &Rc<Self>, folder: &Path) {
        {
            let mut state = self.state.borrow_mut();
            state.folder = folder.to_path_buf();
            state.buffers = Buffers::new();
            state.analysis = None;
            state.tabs.clear();
            state.active = None;
            state.problems.clear();
            state.listed = None;
        }
        while let Some(child) = self.tab_list.first_child() {
            self.tab_list.remove(&child);
        }
        self.editor.show_buffer(&self.editor.new_buffer(""), false);
        self.uses.set_reveal_child(false);
        self.show_body(false);
        self.show_crumbs(&[]);
        self.analyse_now();
        super::tab_strip::page_tabs_changed();
    }

    /// The unsaved texts, for a check that reads what the editor shows.
    /// (Which text the check read comes back with its result, so its
    /// underlines go on the right text.)
    pub fn overlay_for_check(&self) -> Overlay {
        self.state.borrow().buffers.overlay()
    }

    /// What an edit started in the window reads: the open documents'
    /// text, unsaved changes included, over the disk.
    pub fn edit_files(&self) -> Overlay {
        self.state.borrow().buffers.overlay()
    }

    /// The text buffer of a file of the project, when it has a tab; with
    /// `open`, a tab is opened for it first (not shown).
    pub fn buffer_for(self: &Rc<Self>, path: &Path, open: bool) -> Option<gtk::TextBuffer> {
        let document = Document::File(path.to_path_buf());
        let index = if open {
            self.ensure_tab(&document)?
        } else {
            self.state
                .borrow()
                .tabs
                .iter()
                .position(|tab| tab.document == document)?
        };
        self.state
            .borrow()
            .tabs
            .get(index)
            .map(|tab| tab.buffer.clone())
    }

    /// `true` if some document has changes that aren't saved.
    pub fn has_unsaved(&self) -> bool {
        self.state.borrow().buffers.dirty().next().is_some()
    }

    /// How many documents have unsaved changes.
    pub fn unsaved_count(&self) -> usize {
        self.state.borrow().buffers.dirty().count()
    }

    /// Saves every document with unsaved changes.
    pub fn save_all(self: &Rc<Self>) -> bool {
        let result = self.state.borrow_mut().buffers.save_all();
        self.refresh_markers();
        match result {
            Ok(()) => true,
            Err(error) => {
                self.notice(error.to_string());
                false
            }
        }
    }

    /// Saves the document showing.
    pub fn save(self: &Rc<Self>) {
        let document = self.active_document();
        let Some(document) = document else { return };
        if document.is_read_only() {
            self.notice(format!(
                "{} is part of the standard library and can't be changed",
                document.display(&self.state.borrow().folder)
            ));
            return;
        }
        let result = self.state.borrow_mut().buffers.save(&document);
        if let Err(error) = result {
            self.notice(error.to_string());
        }
        self.refresh_markers();
    }

    /// Lays the document showing out with `stave fmt`, as one step that
    /// Undo takes back. Never done unasked.
    pub fn format(self: &Rc<Self>) {
        let Some((document, buffer)) = self.active_tab() else {
            return;
        };
        if document.is_read_only() {
            return;
        }
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        let mut sources = stave_lang::SourceMap::new();
        let Ok(file) = sources.add(document.source_path(), text.as_str()) else {
            return;
        };
        match stave_lang::format_file(file, &text) {
            Ok(formatted) if formatted != text.as_str() => {
                let cursor = buffer.iter_at_mark(&buffer.get_insert());
                let (line, column) = (cursor.line(), cursor.line_offset());
                buffer.begin_user_action();
                let (mut start, mut end) = buffer.bounds();
                buffer.delete(&mut start, &mut end);
                buffer.insert(&mut start, &formatted);
                buffer.end_user_action();
                if let Some(iter) = buffer
                    .iter_at_line_offset(line, column)
                    .or_else(|| buffer.iter_at_line(line))
                {
                    buffer.place_cursor(&iter);
                }
            }
            Ok(_) => self.notice("The file is already laid out".to_owned()),
            Err(error) => self.notice(error.to_string()),
        }
    }

    /// Lets go of unsaved changes (the window is closing and people chose
    /// not to save them).
    pub fn forget_unsaved(&self) {
        self.state.borrow_mut().buffers = Buffers::new();
    }

    /// Ctrl+F in the text: opens its search bar. `false` when the
    /// keyboard isn't in the text.
    pub fn focus_search(&self) -> bool {
        if self.editor.has_focus() {
            self.editor.start_search();
            return true;
        }
        false
    }

    /// The latest analysis of the project, for the finder.
    pub fn analysis(&self) -> Option<Arc<Analysis>> {
        self.state.borrow().analysis.clone()
    }

    /// Takes the latest check's marks for the dots on the tabs.
    pub fn show_marks(&self, marks: stave_app::problems::Marks) {
        self.state.borrow_mut().marks = marks;
        self.refresh_markers();
    }

    /// Takes the last check's problems for the underlines and the cards,
    /// and reloads documents changed on disk by something else.
    pub fn show_result(self: &Rc<Self>, checked: Option<&Checked>) {
        let reloaded = self.reload_from_disk();
        {
            let mut state = self.state.borrow_mut();
            state.problems.clear();
            state.checked_texts.clear();
            if let Some(checked) = checked {
                // Every place a problem points at: where it is, and the
                // places that explain it (the kind's rule in a standard
                // file), each underlined when its file is open.
                for diagnostic in &checked.opened.diagnostics {
                    for label in &diagnostic.labels {
                        let Some(source) = checked.opened.sources.get(label.file) else {
                            continue;
                        };
                        let document = Document::from_source_path(source.path());
                        state
                            .checked_texts
                            .entry(document.clone())
                            .or_insert_with(|| source.text().to_owned());
                        state.problems.push(Problem {
                            document,
                            span: label.span,
                            severity: diagnostic.severity,
                            message: diagnostic.message.clone(),
                            help: diagnostic.help.clone(),
                        });
                    }
                }
            }
        }
        if reloaded {
            self.analyse_now();
        }
        self.refresh_view();
    }

    /// Shows a place: opens its document and puts the cursor on its line
    /// (or on the exact name a "go to" asked for).
    pub fn show_location(self: &Rc<Self>, location: &SourceLocation) {
        self.show_place(location, true);
    }

    /// Shows where a selection made in the 3D view or the model tree is
    /// placed, without telling the window the cursor moved.
    pub fn reveal(self: &Rc<Self>, location: &SourceLocation) {
        self.show_place(location, false);
    }

    /// Asks for screenshot wishes once the names are in.
    pub fn wish(self: &Rc<Self>, wish: Wish) {
        self.state.borrow_mut().wish = Some(wish);
        self.grant_wish();
    }

    /// Follows the light or dark style.
    pub fn restyle(self: &Rc<Self>, dark: bool) {
        self.editor.colour(dark);
        self.refresh_view();
    }

    // ---- inside ----

    fn connect(self: &Rc<Self>, uses_close: &gtk::Button) {
        let page = Rc::downgrade(self);
        uses_close.connect_clicked(move |_| {
            if let Some(page) = page.upgrade() {
                page.uses.set_reveal_child(false);
                page.state.borrow_mut().listed = None;
            }
        });
        self.connect_editor();
    }

    fn connect_editor(self: &Rc<Self>) {
        let view = &self.editor.view;
        let page = Rc::downgrade(self);
        self.editor.connect_lens_uses(move |symbol| {
            if let Some(page) = page.upgrade() {
                page.list_uses(symbol);
            }
        });
        let page = Rc::downgrade(self);
        self.editor.connect_lens_show(move |symbol| {
            let Some(page) = page.upgrade() else { return };
            let analysis = page.state.borrow().analysis.clone();
            let callback = page.callbacks.borrow().on_show_in_3d.clone();
            if let (Some(callback), Some(analysis)) = (callback, analysis) {
                callback(analysis, symbol);
            }
        });
        // Keys: F12, Alt+F12, Shift+F12, Escape; Ctrl+click follows.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let page = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(page) = page.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let alt = modifiers.contains(gtk::gdk::ModifierType::ALT_MASK);
            let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
            match key {
                gtk::gdk::Key::F12 if alt => page.peek_at_cursor(),
                gtk::gdk::Key::F12 if shift => page.list_uses_at_cursor(),
                gtk::gdk::Key::F12 => page.follow_at_cursor(),
                gtk::gdk::Key::Escape if page.editor.has_card() => page.editor.hide_card(),
                gtk::gdk::Key::Escape if page.editor.has_peek() => page.editor.set_peek(None),
                gtk::gdk::Key::Escape if page.editor.has_search() => page.editor.close_search(),
                gtk::gdk::Key::Escape if page.uses.reveals_child() => {
                    page.uses.set_reveal_child(false);
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        view.add_controller(keys);

        let click = gtk::GestureClick::builder()
            .button(gtk::gdk::BUTTON_PRIMARY)
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let page = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(page) = page.upgrade() else { return };
            let control = gesture
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK);
            if !control {
                return;
            }
            let Some(offset) = page.offset_at(x, y) else {
                return;
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            page.follow(offset);
        });
        view.add_controller(click);

        // The card for the word under the pointer, after a short rest.
        let motion = gtk::EventControllerMotion::new();
        let page = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(page) = page.upgrade() {
                page.pointer_moved(x, y);
            }
        });
        let page = Rc::downgrade(self);
        motion.connect_leave(move |_| {
            if let Some(page) = page.upgrade() {
                page.cancel_hover_timer();
                page.hide_hover();
            }
        });
        view.add_controller(motion);
        // Closing the editor (or hiding the window) takes the card too.
        let page = Rc::downgrade(self);
        view.connect_unmap(move |_| {
            if let Some(page) = page.upgrade() {
                page.cancel_hover_timer();
                page.hide_hover();
            }
        });

        // Scrolling and resizing move the overlays with the text.
        let page = Rc::downgrade(self);
        self.editor.vadjustment().connect_changed(move |_| {
            if let Some(page) = page.upgrade() {
                page.settle_overlays();
            }
        });
    }

    /// Keeps the lenses and the peek on their lines for the next half
    /// second, while GTK lays out the text (line heights below the view
    /// are only estimates until then).
    fn settle_overlays(self: &Rc<Self>) {
        let running = {
            let mut state = self.state.borrow_mut();
            let running = state.settling > 0;
            state.settling = 30;
            running
        };
        self.editor.place_overlays();
        if running {
            return;
        }
        let page = Rc::downgrade(self);
        self.editor.view.add_tick_callback(move |_, _| {
            let Some(page) = page.upgrade() else {
                return glib::ControlFlow::Break;
            };
            page.editor.place_overlays();
            let mut state = page.state.borrow_mut();
            state.settling = state.settling.saturating_sub(1);
            if state.settling == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn active_tab(&self) -> Option<(Document, gtk::TextBuffer)> {
        let state = self.state.borrow();
        let tab = state.tabs.get(state.active?)?;
        Some((tab.document.clone(), tab.buffer.clone()))
    }

    fn active_document(&self) -> Option<Document> {
        self.active_tab().map(|(document, _)| document)
    }

    fn active_lines(&self) -> LineStarts {
        let state = self.state.borrow();
        state
            .active
            .and_then(|i| state.tabs.get(i))
            .map(|t| t.lines.clone())
            .unwrap_or_default()
    }

    fn offset_at(&self, x: f64, y: f64) -> Option<u32> {
        self.editor.offset_at(&self.active_lines(), x, y)
    }

    fn cursor(&self) -> u32 {
        self.editor.cursor(&self.active_lines())
    }

    /// The analysis, when it was built from the text showing, with the
    /// file it calls that text.
    fn fresh(&self) -> Option<(Arc<Analysis>, stave_lang::FileId)> {
        let state = self.state.borrow();
        let analysis = state.analysis.clone()?;
        let tab = state.tabs.get(state.active?)?;
        let text = state.buffers.text(&tab.document)?;
        let file = same_text(&analysis, &tab.document, text)?;
        Some((analysis, file))
    }

    /// Opens a document in a tab (or shows its tab) at its top.
    pub fn open_document(self: &Rc<Self>, document: &Document) {
        if let Some(index) = self.ensure_tab(document) {
            self.activate(index);
        }
    }

    fn ensure_tab(self: &Rc<Self>, document: &Document) -> Option<usize> {
        if let Some(index) = self
            .state
            .borrow()
            .tabs
            .iter()
            .position(|t| t.document == *document)
        {
            return Some(index);
        }
        let text = {
            let mut state = self.state.borrow_mut();
            match state.buffers.open(document) {
                Ok(text) => text.to_owned(),
                Err(error) => {
                    drop(state);
                    self.notice(error.to_string());
                    return None;
                }
            }
        };
        let buffer = self.editor.new_buffer(&text);
        let name = document.file_name();
        let tab_label = label(&name, &["stave-code-tab-name"]);
        let dot = gtk::Box::builder()
            .css_classes(["stave-mark"])
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        row.append(&dot);
        row.append(&tab_label);
        let button = gtk::Button::builder()
            .child(&row)
            .css_classes(["flat", "stave-code-tab"])
            .tooltip_text(document.display(&self.state.borrow().folder))
            .build();
        let page = Rc::downgrade(self);
        let key = document.clone();
        button.connect_clicked(move |_| {
            if let Some(page) = page.upgrade() {
                page.open_document(&key);
                let callback = page.callbacks.borrow().on_open.clone();
                if let Some(callback) = callback {
                    callback();
                }
            }
        });
        self.tab_list.append(&button);
        let page = Rc::downgrade(self);
        let key = document.clone();
        buffer.connect_changed(move |buffer| {
            if let Some(page) = page.upgrade() {
                page.text_changed(&key, buffer);
            }
        });
        let page = Rc::downgrade(self);
        buffer.connect_cursor_position_notify(move |_| {
            if let Some(page) = page.upgrade() {
                page.cursor_moved();
            }
        });
        let mut state = self.state.borrow_mut();
        state.tabs.push(Tab {
            document: document.clone(),
            lines: LineStarts::new(&text),
            buffer,
            button,
            label: tab_label,
            dot,
        });
        Some(state.tabs.len().saturating_sub(1))
    }

    /// Shows the open file's text, or the placeholder when none is open.
    fn show_body(&self, open: bool) {
        self.body.set_visible(open);
        self.placeholder.set_visible(!open);
    }

    fn activate(self: &Rc<Self>, index: usize) {
        let found = {
            let mut state = self.state.borrow_mut();
            if state.active == Some(index) {
                None
            } else {
                state.active = Some(index);
                state
                    .tabs
                    .get(index)
                    .map(|t| (t.buffer.clone(), t.document.is_read_only()))
            }
        };
        if let Some((buffer, read_only)) = found {
            self.show_body(true);
            self.hide_hover();
            self.state.borrow_mut().quiet = true;
            self.editor.show_buffer(&buffer, !read_only);
            self.state.borrow_mut().quiet = false;
            self.refresh_markers();
            self.refresh_view();
        }
    }

    fn show_place(self: &Rc<Self>, location: &SourceLocation, report: bool) {
        let document = Document::from_source_path(&location.path);
        let Some(index) = self.ensure_tab(&document) else {
            return;
        };
        self.activate(index);
        let pending = self.state.borrow_mut().pending.take();
        let lines = self.active_lines();
        let line = i32::try_from(location.line.get().saturating_sub(1)).unwrap_or(0);
        let offset = match pending {
            Some((wanted, offset)) if wanted == *location => offset,
            _ => {
                // Already on that line: leave the cursor where it is.
                let (current, _) = lines.position(self.cursor());
                if current == line {
                    self.cursor()
                } else {
                    lines.offset(line, 0)
                }
            }
        };
        self.state.borrow_mut().quiet = !report;
        self.editor.go_to(&lines, offset);
        self.state.borrow_mut().quiet = false;
        if self.editor.view.is_mapped() {
            self.editor.view.grab_focus();
        }
    }

    fn text_changed(self: &Rc<Self>, document: &Document, buffer: &gtk::TextBuffer) {
        let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
        let (was_dirty, dirty) = {
            let mut state = self.state.borrow_mut();
            if let Some(tab) = state.tabs.iter_mut().find(|t| t.document == *document) {
                tab.lines = LineStarts::new(&text);
            }
            let was_dirty = state.buffers.is_dirty(document);
            if state.buffers.edit(document, &text).is_err() {
                return;
            }
            (was_dirty, state.buffers.is_dirty(document))
        };
        self.editor.set_peek(None);
        self.hide_hover();
        if was_dirty != dirty {
            self.refresh_markers();
        }
        self.schedule_analysis();
        self.schedule_check();
        self.settle_overlays();
        self.state.borrow_mut().relink = true;
    }

    fn schedule_analysis(self: &Rc<Self>) {
        let page = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(ANALYSE_DELAY, move || {
            if let Some(page) = page.upgrade() {
                page.state.borrow_mut().analyse_timer = None;
                page.analyse_now();
            }
        });
        if let Some(old) = self.state.borrow_mut().analyse_timer.replace(timer) {
            old.remove();
        }
    }

    fn schedule_check(self: &Rc<Self>) {
        let page = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(CHECK_DELAY, move || {
            let Some(page) = page.upgrade() else { return };
            page.state.borrow_mut().check_timer = None;
            let callback = page.callbacks.borrow().on_check.clone();
            if let Some(callback) = callback {
                callback();
            }
        });
        if let Some(old) = self.state.borrow_mut().check_timer.replace(timer) {
            old.remove();
        }
    }

    /// Works the names out again from the buffers, on a worker thread.
    fn analyse_now(self: &Rc<Self>) {
        let (folder, overlay, generation) = {
            let mut state = self.state.borrow_mut();
            if state.analysing {
                state.analyse_again = true;
                return;
            }
            if state.folder.as_os_str().is_empty() {
                return;
            }
            state.analysing = true;
            state.generation = state.generation.wrapping_add(1);
            (
                state.folder.clone(),
                state.buffers.overlay(),
                state.generation,
            )
        };
        let page = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let loaded = gio::spawn_blocking(move || {
                let analysis = Analysis::load(&folder, &overlay).ok();
                (folder, analysis)
            })
            .await;
            let Some(page) = page.upgrade() else { return };
            let again = {
                let mut state = page.state.borrow_mut();
                state.analysing = false;
                if let Ok((folder, Some(analysis))) = loaded
                    && folder == state.folder
                    && generation == state.generation
                {
                    state.analysis = Some(Arc::new(analysis));
                }
                std::mem::take(&mut state.analyse_again)
            };
            if again {
                page.analyse_now();
            }
            page.refresh_view();
            page.grant_wish();
        });
    }

    /// Repaints the document showing from the analysis and the problems:
    /// colours, underlines, lenses, breadcrumbs, the line.
    fn refresh_view(self: &Rc<Self>) {
        let Some((document, buffer)) = self.active_tab() else {
            return;
        };
        let lines = self.active_lines();
        let fresh = self.fresh();
        if let Some((analysis, file)) = &fresh {
            let runs: Vec<(Run, TokenClass)> = analysis
                .tokens(*file)
                .into_iter()
                .map(|t| {
                    (
                        Run {
                            start: t.span.start(),
                            end: t.span.end(),
                        },
                        t.class,
                    )
                })
                .collect();
            self.editor.paint(&buffer, &lines, &runs);
            self.show_lenses(analysis, *file);
        }
        // The check's problems are underlined whether or not the names
        // are worked out yet: they need only the text the check read.
        let mut problems: Vec<(Run, Severity)> = fresh
            .iter()
            .flat_map(|(analysis, file)| {
                analysis
                    .diagnostics(*file)
                    .filter_map(|d| {
                        let span = d.primary()?.span;
                        Some((
                            Run {
                                start: span.start(),
                                end: span.end(),
                            },
                            d.severity,
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        problems.extend(self.check_problems(&document).into_iter().map(|p| {
            (
                Run {
                    start: p.span.start(),
                    end: p.span.end(),
                },
                p.severity,
            )
        }));
        self.editor.underline(&buffer, &lines, &problems);
        let relink = fresh.is_some() && std::mem::take(&mut self.state.borrow_mut().relink);
        self.cursor_moved_now(relink);
    }

    /// The last check's problems in a document, while it still reads as
    /// the check read it (after an edit their spans would point at the
    /// wrong text; the next check brings them back).
    fn check_problems(&self, document: &Document) -> Vec<Problem> {
        let state = self.state.borrow();
        let current = state.buffers.text(document);
        let checked = state.checked_texts.get(document).map(String::as_str);
        if current.is_none() || current != checked {
            return Vec::new();
        }
        state
            .problems
            .iter()
            .filter(|p| p.document == *document)
            .cloned()
            .collect()
    }

    fn show_lenses(self: &Rc<Self>, analysis: &Arc<Analysis>, file: stave_lang::FileId) {
        let lenses: Vec<LensLine> = analysis
            .lenses(file)
            .into_iter()
            .filter_map(|lens| {
                Some(LensLine {
                    line: i32::try_from(lens.location.line.saturating_sub(1)).ok()?,
                    uses: lens.uses_text(),
                    shows_in_3d: lens.shows_in_3d(),
                    symbol: lens.symbol,
                    name: lens.name,
                })
            })
            .collect();
        self.editor.set_lenses(&lenses);
        self.settle_overlays();
    }

    fn cursor_moved(self: &Rc<Self>) {
        self.cursor_moved_now(true);
    }

    /// The breadcrumbs, the line, the marked uses, and (after a rest) the
    /// link to the 3D view follow the cursor.
    fn cursor_moved_now(self: &Rc<Self>, report: bool) {
        let Some((_, buffer)) = self.active_tab() else {
            return;
        };
        let cursor_iter = buffer.iter_at_mark(&buffer.get_insert());
        self.line
            .set_label(&format!("Line {}", cursor_iter.line().saturating_add(1)));
        let lines = self.active_lines();
        let offset = self.cursor();
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        let position = FilePosition { file, offset };
        let crumbs: Vec<String> = analysis
            .breadcrumbs(position)
            .into_iter()
            .map(|c| c.label)
            .collect();
        self.show_crumbs(&crumbs);
        let symbol = analysis.symbol_at(position);
        let (declared, uses) = match symbol.and_then(|s| analysis.uses(s)) {
            Some(uses) => {
                let mut declared = None;
                let mut runs = Vec::new();
                for found in uses.uses.iter().filter(|u| u.location.file == file) {
                    let run = Run {
                        start: found.location.span.start(),
                        end: found.location.span.end(),
                    };
                    if found.kind == stave_ide::UseKind::Declared {
                        declared = Some(run);
                    } else {
                        runs.push(run);
                    }
                }
                (declared, runs)
            }
            None => (None, Vec::new()),
        };
        self.editor.mark_uses(&buffer, &lines, declared, &uses);
        if report && !self.state.borrow().quiet {
            let page = Rc::downgrade(self);
            let timer = glib::timeout_add_local_once(LINK_DELAY, move || {
                let Some(page) = page.upgrade() else { return };
                page.state.borrow_mut().link_timer = None;
                let found = match page.fresh() {
                    Some((analysis, file)) => {
                        let position = FilePosition {
                            file,
                            offset: page.cursor(),
                        };
                        match analysis.symbol_at(position) {
                            Some(symbol) => CursorName::Name(analysis, symbol),
                            None if analysis.is_unresolved(position) => CursorName::Unresolved,
                            None => CursorName::Other,
                        }
                    }
                    None => CursorName::Other,
                };
                let callback = page.callbacks.borrow().on_link.clone();
                if let Some(callback) = callback {
                    callback(found);
                }
            });
            if let Some(old) = self.state.borrow_mut().link_timer.replace(timer) {
                old.remove();
            }
        }
    }

    fn show_crumbs(&self, crumbs: &[String]) {
        while let Some(child) = self.crumbs.first_child() {
            self.crumbs.remove(&child);
        }
        let last = crumbs.len().saturating_sub(1);
        for (index, crumb) in crumbs.iter().enumerate() {
            if index > 0 {
                self.crumbs.append(&label("›", &["stave-crumb-separator"]));
            }
            let classes: &[&str] = if index == last && index > 0 {
                &["stave-crumb", "current"]
            } else {
                &["stave-crumb"]
            };
            let crumb = label(crumb, classes);
            crumb.set_ellipsize(gtk::pango::EllipsizeMode::End);
            crumb.set_max_width_chars(24);
            self.crumbs.append(&crumb);
        }
    }

    /// Marks unsaved tabs, the active one, each file's worst problem, and
    /// names the active file over the text.
    fn refresh_markers(&self) {
        let state = self.state.borrow();
        for (index, tab) in state.tabs.iter().enumerate() {
            let dirty = state.buffers.is_dirty(&tab.document);
            let name = tab.document.file_name();
            tab.label.set_label(&if dirty {
                format!("{name} •")
            } else {
                name.clone()
            });
            if Some(index) == state.active {
                tab.button.add_css_class("active");
            } else {
                tab.button.remove_css_class("active");
            }
            let mark = state.marks.file(&tab.document.source_path());
            super::style::show_mark(&tab.dot, mark);
            tab.button.set_tooltip_text(Some(&if dirty {
                format!(
                    "{} — not saved (Ctrl+S saves)",
                    tab.document.display(&state.folder)
                )
            } else {
                tab.document.display(&state.folder)
            }));
        }
        drop(state);
        super::tab_strip::page_tabs_changed();
    }

    /// The open files as tabs for the strip of the frame holding this
    /// panel (beside the Model): name, worst problem, unsaved or not, the
    /// current one; showing one opens it here, as its old button did.
    pub fn page_tabs(self: &Rc<Self>) -> Vec<super::tab_strip::PageTab> {
        let state = self.state.borrow();
        state
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let dirty = state.buffers.is_dirty(&tab.document);
                let shown = tab.document.display(&state.folder);
                let page = Rc::downgrade(self);
                let key = tab.document.clone();
                super::tab_strip::PageTab {
                    title: tab.document.file_name(),
                    tooltip: if dirty {
                        format!("{shown} — not saved (Ctrl+S saves)")
                    } else {
                        shown
                    },
                    unsaved: dirty,
                    mark: state.marks.file(&tab.document.source_path()),
                    active: Some(index) == state.active,
                    show: Rc::new(move || {
                        if let Some(page) = page.upgrade() {
                            page.open_document(&key);
                            let callback = page.callbacks.borrow().on_open.clone();
                            if let Some(callback) = callback {
                                callback();
                            }
                        }
                    }),
                }
            })
            .collect()
    }

    // ---- navigation ----

    /// Goes to a place through the window, so Back returns here.
    fn go(self: &Rc<Self>, location: &Location) {
        let Some(line) = LineNumber::from_usize(location.line) else {
            return;
        };
        let to = SourceLocation {
            path: location.document.source_path(),
            line,
        };
        let from = self.active_document().and_then(|document| {
            let (line, _) = self.active_lines().position(self.cursor());
            Some(SourceLocation {
                path: document.source_path(),
                line: LineNumber::from_usize(usize::try_from(line).ok()?.saturating_add(1))?,
            })
        });
        self.state.borrow_mut().pending = Some((to.clone(), location.span.start()));
        let callback = self.callbacks.borrow().on_go.clone();
        match callback {
            Some(callback) => callback(from, to),
            None => self.show_location(&to),
        }
    }

    /// Follows the name at an offset to its declaration.
    fn follow(self: &Rc<Self>, offset: u32) {
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        match analysis.definition(FilePosition { file, offset }) {
            Some(location) => self.go(&location),
            None => self
                .notice("This word isn't declared anywhere: it is part of the language".to_owned()),
        }
    }

    fn follow_at_cursor(self: &Rc<Self>) {
        self.follow(self.cursor());
    }

    fn list_uses_at_cursor(self: &Rc<Self>) {
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        if let Some(symbol) = analysis.symbol_at(FilePosition {
            file,
            offset: self.cursor(),
        }) {
            self.list_uses(symbol);
        }
    }

    /// Lists every use of a symbol in the panel under the editor.
    fn list_uses(self: &Rc<Self>, symbol: SymbolId) {
        let Some((analysis, _)) = self.fresh() else {
            return;
        };
        let Some(uses) = analysis.uses(symbol) else {
            return;
        };
        let folder = analysis.folder().to_path_buf();
        while let Some(child) = self.uses_list.first_child() {
            self.uses_list.remove(&child);
        }
        let count = uses.count();
        self.uses_title.set_markup(&format!(
            "<b>{}</b> · {}",
            escape(&uses.name),
            match count {
                0 => "no uses".to_owned(),
                1 => "1 use".to_owned(),
                n => format!("{n} uses"),
            }
        ));
        for found in uses.uses {
            let place = label(&found.location.label(&folder), &["stave-use-place"]);
            let what = label(&found.description, &["stave-use-what"]);
            let line = label(&found.line_text, &["stave-use-line"]);
            line.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.set_hexpand(true);
            let row_box = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(12)
                .css_classes(["stave-use"])
                .build();
            row_box.append(&place);
            row_box.append(&what);
            row_box.append(&line);
            let row = gtk::ListBoxRow::builder()
                .child(&row_box)
                .activatable(true)
                .build();
            let page = Rc::downgrade(self);
            let location = found.location.clone();
            let click = gtk::GestureClick::new();
            click.connect_released(move |_, _, _, _| {
                if let Some(page) = page.upgrade() {
                    page.go(&location);
                }
            });
            row.add_controller(click);
            self.uses_list.append(&row);
        }
        self.state.borrow_mut().listed = Some(symbol);
        self.uses.set_reveal_child(true);
    }

    /// Opens a peek of the definition of the name under the cursor, under
    /// its line.
    fn peek_at_cursor(self: &Rc<Self>) {
        if self.editor.has_peek() {
            self.editor.set_peek(None);
            return;
        }
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        let offset = self.cursor();
        let position = FilePosition { file, offset };
        let Some(symbol) = analysis
            .symbol_at(position)
            .and_then(|s| analysis.symbol(s))
        else {
            return;
        };
        let Some(location) = analysis.declaration(symbol.id) else {
            return;
        };
        let peek = self.peek_widget(&analysis, symbol, &location);
        let (line, _) = self.active_lines().position(offset);
        self.editor.set_peek(Some((peek.upcast(), line)));
        self.settle_overlays();
    }

    fn peek_widget(
        self: &Rc<Self>,
        analysis: &Arc<Analysis>,
        symbol: &stave_ide::Symbol,
        location: &Location,
    ) -> gtk::Box {
        let folder = analysis.folder();
        let head = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .css_classes(["stave-peek-head"])
            .build();
        head.append(&label(
            &location.document.display(folder),
            &["stave-peek-path"],
        ));
        let what = label(
            &format!("· {} {}", symbol.kind.word(), symbol.name),
            &["stave-peek-what"],
        );
        what.set_hexpand(true);
        what.set_ellipsize(gtk::pango::EllipsizeMode::End);
        head.append(&what);
        let open = super::style::as_link(
            gtk::Button::builder()
                .label("Open")
                .css_classes(["flat", "stave-lens-link"])
                .tooltip_text("Open the definition (F12)")
                .build(),
        );
        let page = Rc::downgrade(self);
        let target = location.clone();
        open.connect_clicked(move |_| {
            if let Some(page) = page.upgrade() {
                page.editor.set_peek(None);
                page.go(&target);
            }
        });
        head.append(&open);
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-peek-body"])
            .build();
        // From the first doc line to a few lines past the name.
        let first_line = analysis
            .location(
                symbol.file,
                Span::new(symbol.span.start(), symbol.span.start()),
            )
            .map_or(location.line, |l| l.line);
        let last_line = location.line.saturating_add(4);
        let text = analysis.text(symbol.file).unwrap_or_default();
        let tokens = analysis.tokens(symbol.file);
        let dark = adw::StyleManager::default().is_dark();
        for (index, line_text) in text.lines().enumerate() {
            let number = index.saturating_add(1);
            if number < first_line || number > last_line {
                continue;
            }
            let start = analysis.line_start(symbol.file, number).unwrap_or(0);
            let markup = markup_line(line_text, start, &tokens, dark);
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .css_classes(if number == location.line {
                    vec!["stave-peek-line", "current"]
                } else {
                    vec!["stave-peek-line"]
                })
                .build();
            let number_label = label(&number.to_string(), &["stave-peek-number"]);
            number_label.set_xalign(1.0);
            number_label.set_width_chars(3);
            row.append(&number_label);
            let code = gtk::Label::builder()
                .use_markup(true)
                .label(&markup)
                .xalign(0.0)
                .css_classes(["stave-peek-code"])
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            row.append(&code);
            body.append(&row);
        }
        let peek = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-peek"])
            .build();
        peek.append(&head);
        peek.append(&body);
        peek
    }

    // ---- the hover card ----

    fn pointer_moved(self: &Rc<Self>, x: f64, y: f64) {
        self.cancel_hover_timer();
        let offset = self.offset_at(x, y);
        // Off the word the card is about (another word, past the end of a
        // line, a blank line): the card goes now, not after the rest.
        let shown = self.state.borrow().hover_span;
        if let Some(span) = shown
            && !offset.is_some_and(|offset| span.start() <= offset && offset <= span.end())
        {
            self.hide_hover();
        }
        let page = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(HOVER_DELAY, move || {
            let Some(page) = page.upgrade() else { return };
            page.state.borrow_mut().hover_timer = None;
            if let Some(offset) = offset {
                page.show_hover(offset);
            }
        });
        self.state.borrow_mut().hover_timer = Some(timer);
    }

    fn cancel_hover_timer(&self) {
        if let Some(old) = self.state.borrow_mut().hover_timer.take() {
            old.remove();
        }
    }

    fn hide_hover(&self) {
        self.state.borrow_mut().hover_span = None;
        self.editor.hide_card();
    }

    /// Shows the card for the word at an offset: its problem, or what it
    /// is with its doc comment and where it is declared.
    fn show_hover(self: &Rc<Self>, offset: u32) {
        self.hide_hover();
        let Some((document, buffer)) = self.active_tab() else {
            return;
        };
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["stave-hover"])
            .width_request(280)
            .build();
        let mut anchor: Option<Span> = None;
        let fast: Vec<Problem> = analysis
            .diagnostics(file)
            .filter_map(|d| {
                Some(Problem {
                    document: document.clone(),
                    span: d.primary()?.span,
                    severity: d.severity,
                    message: d.message.clone(),
                    help: d.help.clone(),
                })
            })
            .collect();
        for problem in fast.iter().chain(self.check_problems(&document).iter()) {
            let covers = problem.span.start() <= offset
                && (offset < problem.span.end()
                    || (problem.span.start() == problem.span.end()
                        && offset <= problem.span.end().saturating_add(1)));
            if !covers {
                continue;
            }
            let (word, class) = match problem.severity {
                Severity::Error => ("Error", "error"),
                Severity::Warning => ("Warning", "warning"),
                Severity::Note => ("Note", "note"),
            };
            card.append(&label(word, &["stave-hover-kind", class]));
            let message = label(&problem.message, &["stave-hover-message"]);
            message.set_wrap(true);
            message.set_max_width_chars(48);
            card.append(&message);
            if let Some(help) = &problem.help {
                let help = label(&format!("Help: {help}"), &["stave-hover-help"]);
                help.set_wrap(true);
                help.set_max_width_chars(48);
                card.append(&help);
            }
            anchor = Some(anchor.map_or(problem.span, |span| span.to(problem.span)));
        }
        if anchor.is_none()
            && let Some(hover) = analysis.hover(FilePosition { file, offset })
        {
            card.append(&label(&hover.kind.to_uppercase(), &["stave-hover-kind"]));
            let signature = label(&hover.signature, &["stave-hover-signature"]);
            signature.set_wrap(true);
            signature.set_max_width_chars(48);
            card.append(&signature);
            if let Some(docs) = &hover.docs {
                let docs = label(docs, &["stave-hover-docs"]);
                docs.set_wrap(true);
                docs.set_max_width_chars(48);
                card.append(&docs);
            }
            if let Some(location) = &hover.location {
                card.append(&label(
                    &format!("Defined in {}", location.label(analysis.folder())),
                    &["stave-hover-where"],
                ));
            }
            anchor = Some(hover.span);
        }
        let Some(anchor) = anchor else {
            return;
        };
        self.state.borrow_mut().hover_span = Some(anchor);
        let lines = self.active_lines();
        let iter = super::editor::iter_at(&buffer, &lines, anchor.start());
        let rect = self.editor.view.iter_location(&iter);
        self.editor.show_card(
            card.upcast_ref(),
            rect.x(),
            rect.y().saturating_add(rect.height()).saturating_add(6),
        );
    }

    /// Reloads open documents changed on disk; unsaved changes are kept.
    /// `true` if any buffer changed.
    fn reload_from_disk(self: &Rc<Self>) -> bool {
        let documents: Vec<Document> = self.state.borrow().buffers.documents().cloned().collect();
        let mut changed = false;
        for document in documents {
            let reloaded = self.state.borrow_mut().buffers.reload(&document);
            match reloaded {
                Ok(stave_ide::Reload::Reloaded) => {
                    let found = {
                        let state = self.state.borrow();
                        state
                            .tabs
                            .iter()
                            .find(|t| t.document == document)
                            .map(|t| t.buffer.clone())
                            .zip(state.buffers.text(&document).map(str::to_owned))
                    };
                    if let Some((buffer, text)) = found {
                        let cursor = buffer.iter_at_mark(&buffer.get_insert());
                        let line = cursor.line();
                        self.state.borrow_mut().quiet = true;
                        buffer.set_text(&text);
                        if let Some(iter) = buffer.iter_at_line(line) {
                            buffer.place_cursor(&iter);
                        }
                        self.state.borrow_mut().quiet = false;
                    }
                    changed = true;
                }
                Ok(stave_ide::Reload::KeptChanges) => {
                    self.notice(format!(
                        "{} changed on disk; your unsaved changes are kept",
                        document.file_name()
                    ));
                }
                Ok(stave_ide::Reload::Unchanged) | Err(_) => {}
            }
        }
        changed
    }

    // ---- screenshots ----

    fn grant_wish(self: &Rc<Self>) {
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        let Some(wish) = self.state.borrow_mut().wish.take() else {
            return;
        };
        if let Some(word) = &wish.cursor {
            let lines = self.active_lines();
            let (line, _) = lines.position(self.cursor());
            let start = lines.offset(line, 0);
            if let Some(found) = analysis
                .text(file)
                .and_then(|t| t.get(usize::try_from(start).unwrap_or(0)..))
                .and_then(|rest| rest.lines().next()?.find(word.as_str()))
            {
                let offset = start.saturating_add(u32::try_from(found).unwrap_or(0));
                self.state.borrow_mut().quiet = true;
                self.editor.go_to(&lines, offset);
                self.state.borrow_mut().quiet = false;
                self.cursor_moved();
            }
        }
        let page = self.clone();
        // Once the text is laid out.
        glib::timeout_add_local_once(Duration::from_millis(150), move || {
            if let Some(text) = &wish.typing {
                let (_, buffer) = page.active_tab().unzip();
                if let Some(buffer) = buffer {
                    // A key at a time, so undo groups it as it groups typing.
                    let mut key = [0; 4];
                    for character in text.chars() {
                        buffer.insert_interactive_at_cursor(character.encode_utf8(&mut key), true);
                    }
                }
            }
            if wish.undo
                && let Some((_, buffer)) = page.active_tab()
            {
                buffer.undo();
            }
            if let Some(text) = &wish.search {
                page.editor.find(text);
            }
            if wish.follow {
                page.follow_at_cursor();
            }
            if wish.uses {
                page.list_uses_at_cursor();
            }
            if wish.peek {
                page.peek_at_cursor();
            }
            if wish.hover {
                page.show_hover(page.cursor());
            }
        });
    }
}

/// One line of source as Pango markup, coloured by the file's tokens.
fn markup_line(text: &str, start: u32, tokens: &[stave_ide::SemanticToken], dark: bool) -> String {
    let end = start.saturating_add(u32::try_from(text.len()).unwrap_or(0));
    let mut out = String::new();
    let mut at = 0usize;
    for token in tokens
        .iter()
        .filter(|t| t.span.start() >= start && t.span.end() <= end)
    {
        let (Ok(from), Ok(to)) = (
            usize::try_from(token.span.start().saturating_sub(start)),
            usize::try_from(token.span.end().saturating_sub(start)),
        ) else {
            continue;
        };
        if from < at {
            continue;
        }
        if let Some(plain) = text.get(at..from) {
            out.push_str(&escape(plain));
        }
        let piece = escape(text.get(from..to).unwrap_or(""));
        let colour = match token.class {
            TokenClass::Keyword => Some(if dark { "#dc8add" } else { "#9141ac" }),
            TokenClass::Comment | TokenClass::DocComment => {
                Some(if dark { "#9a9996" } else { "#6f6e73" })
            }
            TokenClass::Text => Some(if dark { "#8ff0a4" } else { "#1b7a4c" }),
            TokenClass::Number => Some(if dark { "#ffa348" } else { "#c64600" }),
            TokenClass::Function | TokenClass::EnumWord => {
                Some(if dark { "#78aeed" } else { "#1c71d8" })
            }
            TokenClass::Type => Some(if dark { "#99c1f1" } else { "#1a5fb4" }),
            _ => None,
        };
        let italic = matches!(token.class, TokenClass::Comment | TokenClass::DocComment);
        match colour {
            Some(colour) if italic => {
                out.push_str(&format!(
                    "<span foreground=\"{colour}\" style=\"italic\">{piece}</span>"
                ));
            }
            Some(colour) => out.push_str(&format!("<span foreground=\"{colour}\">{piece}</span>")),
            None => out.push_str(&piece),
        }
        at = to;
    }
    if let Some(rest) = text.get(at..) {
        out.push_str(&escape(rest));
    }
    out
}
