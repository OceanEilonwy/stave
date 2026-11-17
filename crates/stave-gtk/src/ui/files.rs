//! The Files view (FilesMode.dc.html): the project's files and the
//! standard library in a sidebar with "Go to file or name", editor tabs
//! with a Text | Both switch, breadcrumbs, the editor, the uses panel and
//! a footer; beside it, in Both, the 3D view, linked to the text.
//!
//! Everything it knows about the text comes from `stave-ide`: the open
//! buffers ([`Buffers`]), and an [`Analysis`] built again on a worker
//! thread a moment after each change, from the buffers as typed. The
//! window decides what a name means in the design and owns the history:
//! this page tells it where the cursor is ([`FilesPage::connect_link`]),
//! where to go ([`FilesPage::connect_go`]) and when typing has paused
//! ([`FilesPage::connect_check`]), and it calls back with a place to show.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
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

use super::editor::{Editor, LineStarts, Run};

/// How long typing must pause before the names are worked out again.
const ANALYSE_DELAY: Duration = Duration::from_millis(150);
/// How long typing must pause before the design is checked again.
const CHECK_DELAY: Duration = Duration::from_millis(700);
/// How long the cursor must rest on a name before the 3D view follows.
const LINK_DELAY: Duration = Duration::from_millis(120);
/// How long the pointer must rest on a word before its card shows.
const HOVER_DELAY: Duration = Duration::from_millis(450);
/// The most results "Go to file or name" lists.
const SEARCH_LIMIT: usize = 40;

/// The editor alone, or beside the 3D view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The editor only.
    Text,
    /// The editor beside the 3D view.
    #[default]
    Both,
}

impl Mode {
    fn word(self) -> &'static str {
        match self {
            Mode::Text => "text",
            Mode::Both => "both",
        }
    }
}

/// What the screenshot asks the Files view to show, once the names are in.
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
}

type LinkCallback = Rc<dyn Fn(Option<(Arc<Analysis>, SymbolId)>)>;
type GoCallback = Rc<dyn Fn(Option<SourceLocation>, SourceLocation)>;
type SymbolCallback = Rc<dyn Fn(Arc<Analysis>, SymbolId)>;

#[derive(Default)]
struct Callbacks {
    on_link: Option<LinkCallback>,
    on_go: Option<GoCallback>,
    on_check: Option<Rc<dyn Fn()>>,
    on_show_in_3d: Option<SymbolCallback>,
    on_mode: Option<Rc<dyn Fn(Mode)>>,
    on_notice: Option<Rc<dyn Fn(String)>>,
}

/// One open document.
struct Tab {
    document: Document,
    buffer: gtk::TextBuffer,
    lines: LineStarts,
    button: gtk::Button,
    label: gtk::Label,
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

/// What a sidebar row does.
#[derive(Debug, Clone)]
enum RowTarget {
    Nothing,
    Open(Document),
    Expand(&'static str),
    Found(Location),
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
    /// The texts the running check reads.
    check_texts: Vec<(Document, String)>,
    /// Changes made by the page itself: don't report them.
    quiet: bool,
    /// The exact place to put the cursor when the next place shown is
    /// this location.
    pending: Option<(SourceLocation, u32)>,
    analyse_timer: Option<glib::SourceId>,
    check_timer: Option<glib::SourceId>,
    link_timer: Option<glib::SourceId>,
    hover_timer: Option<glib::SourceId>,
    hover: Option<gtk::Widget>,
    mode: Mode,
    expanded: BTreeSet<&'static str>,
    rows: Vec<RowTarget>,
    wish: Option<Wish>,
    /// The symbol whose uses the panel lists.
    listed: Option<SymbolId>,
}

/// The Files view.
pub struct FilesPage {
    /// The page, for the view stack.
    pub root: adw::OverlaySplitView,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    tab_row: gtk::Box,
    mode_toggle: adw::ToggleGroup,
    crumbs: gtk::Box,
    editor: Editor,
    uses: gtk::Revealer,
    uses_title: gtk::Label,
    uses_list: gtk::ListBox,
    line: gtk::Label,
    paned: gtk::Paned,
    /// Where the window puts the 3D view in Both.
    pub view_slot: gtk::Overlay,
    chip: gtk::Box,
    chip_label: gtk::Label,
    state: RefCell<State>,
    callbacks: RefCell<Callbacks>,
    placed: Cell<bool>,
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
    /// An empty Files view.
    pub fn new() -> Rc<FilesPage> {
        // The sidebar.
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Go to file or name")
            .css_classes(["stave-search"])
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["stave-tree", "stave-files"])
            .build();
        let sidebar = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .css_classes(["stave-sidebar"])
            .build();
        sidebar.append(&search);
        sidebar.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&list)
                .vexpand(true)
                .build(),
        );

        // The tab row: tabs, then Text | Both.
        let tab_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(2)
            .hexpand(true)
            .build();
        let mode_toggle = adw::ToggleGroup::builder()
            .css_classes(["stave-mode"])
            .valign(gtk::Align::Center)
            .build();
        for (mode, title) in [(Mode::Text, "Text"), (Mode::Both, "Both")] {
            mode_toggle.add(
                adw::Toggle::builder()
                    .name(mode.word())
                    .label(title)
                    .tooltip(match mode {
                        Mode::Text => "The text alone",
                        Mode::Both => "The text beside the 3D view",
                    })
                    .build(),
            );
        }
        mode_toggle.set_active_name(Some(Mode::Both.word()));
        let tabs = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-tabs"])
            .build();
        tabs.append(&tab_row);
        tabs.append(&mode_toggle);

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
        let line = label("", &["stave-footer-line"]);
        let footer = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .css_classes(["stave-editor-footer"])
            .build();
        footer.append(&hint);
        footer.append(&line);

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-editor-column"])
            .width_request(320)
            .build();
        column.append(&tabs);
        column.append(&crumbs);
        column.append(&editor.root);
        column.append(&uses);
        column.append(&footer);

        // The 3D view's place, with the chip that says it follows the text.
        let chip_label = label("", &[]);
        let chip = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::End)
            .margin_start(12)
            .margin_bottom(12)
            .css_classes(["stave-chip"])
            .visible(false)
            .build();
        chip.append(&super::icons::PathIcon::new(super::icons::LINK, 16));
        chip.append(&chip_label);
        let view_slot = gtk::Overlay::builder()
            .css_classes(["stave-viewport"])
            .width_request(300)
            .build();
        view_slot.add_overlay(&chip);

        let paned = gtk::Paned::builder()
            .orientation(gtk::Orientation::Horizontal)
            .start_child(&column)
            .end_child(&view_slot)
            .resize_start_child(true)
            .resize_end_child(true)
            .shrink_start_child(false)
            .shrink_end_child(false)
            .wide_handle(false)
            .css_classes(["stave-files-split"])
            .build();

        let root = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&paned)
            .min_sidebar_width(200.0)
            .max_sidebar_width(230.0)
            .sidebar_width_fraction(0.16)
            .show_sidebar(true)
            .build();

        let page = Rc::new(FilesPage {
            root,
            search,
            list,
            tab_row,
            mode_toggle,
            crumbs,
            editor,
            uses,
            uses_title,
            uses_list,
            line,
            paned,
            view_slot,
            chip,
            chip_label,
            state: RefCell::new(State::default()),
            callbacks: RefCell::new(Callbacks::default()),
            placed: Cell::new(false),
        });
        page.connect(&uses_close);
        page
    }

    // ---- what the window hears ----

    /// The cursor rests on a name (or on nothing that names an item).
    pub fn connect_link(&self, callback: impl Fn(Option<(Arc<Analysis>, SymbolId)>) + 'static) {
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

    /// Text | Both changed.
    pub fn connect_mode(&self, callback: impl Fn(Mode) + 'static) {
        self.callbacks.borrow_mut().on_mode = Some(Rc::new(callback));
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

    /// Text or Both.
    pub fn mode(&self) -> Mode {
        self.state.borrow().mode
    }

    /// Switches Text | Both.
    pub fn set_mode(&self, mode: Mode) {
        self.mode_toggle.set_active_name(Some(mode.word()));
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
        while let Some(child) = self.tab_row.first_child() {
            self.tab_row.remove(&child);
        }
        self.editor.show_buffer(&self.editor.new_buffer(""), false);
        self.uses.set_reveal_child(false);
        self.show_crumbs(&[]);
        self.fill_sidebar();
        self.analyse_now();
    }

    /// The unsaved texts, for a check that reads what the editor shows.
    /// Remembers them, so the check's underlines go on the right text.
    pub fn overlay_for_check(&self) -> Overlay {
        let mut state = self.state.borrow_mut();
        let texts: Vec<(Document, String)> = state
            .buffers
            .documents()
            .filter_map(|d| Some((d.clone(), state.buffers.text(d)?.to_owned())))
            .collect();
        state.check_texts = texts;
        state.buffers.overlay()
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

    /// The split between the editor and the 3D view, for the window's
    /// narrow layout.
    pub fn paned(&self) -> &gtk::Paned {
        &self.paned
    }

    /// Puts the keyboard in the search field.
    pub fn focus_search(&self) {
        self.root.set_show_sidebar(true);
        self.search.grab_focus();
    }

    /// Takes the last check's problems for the underlines and the cards,
    /// and reloads documents changed on disk by something else.
    pub fn show_result(self: &Rc<Self>, checked: Option<&Checked>) {
        let reloaded = self.reload_from_disk();
        {
            let mut state = self.state.borrow_mut();
            state.problems.clear();
            if let Some(checked) = checked {
                for diagnostic in &checked.opened.diagnostics {
                    let Some(label) = diagnostic.primary() else {
                        continue;
                    };
                    let Some(source) = checked.opened.sources.get(label.file) else {
                        continue;
                    };
                    let document = Document::from_source_path(source.path());
                    // Only problems in text that still reads as checked.
                    let checked_text = state
                        .check_texts
                        .iter()
                        .find(|(d, _)| *d == document)
                        .map(|(_, t)| t.as_str());
                    let current = state.buffers.text(&document);
                    if current.is_some() && checked_text.is_some() && current != checked_text {
                        continue;
                    }
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
        if reloaded {
            self.analyse_now();
        }
        self.refresh_view();
    }

    /// Shows a place: opens its document and puts the cursor on its line
    /// (or on the exact name a "go to" asked for).
    pub fn show_location(self: &Rc<Self>, location: &SourceLocation) {
        self.show_place(location, true);
        self.set_chip(None);
    }

    /// Shows where a selection made in the 3D view or the parts tree is
    /// placed, with the chip's words, without telling the window the
    /// cursor moved.
    pub fn reveal(self: &Rc<Self>, location: &SourceLocation, chip: Option<&str>) {
        self.show_place(location, false);
        self.set_chip(chip);
    }

    /// Keeps the chip clear of a bar at the bottom of the 3D view (the
    /// motion bar) while that bar shows.
    pub fn keep_chip_above(&self, bar: &impl IsA<gtk::Widget>) {
        bar.bind_property("visible", &self.chip, "margin-bottom")
            .transform_to(|_, visible: bool| Some(if visible { 84 } else { 12 }))
            .sync_create()
            .build();
    }

    /// Shows or hides the chip on the 3D view.
    pub fn set_chip(&self, chip: Option<&str>) {
        match chip {
            Some(text) => {
                self.chip_label.set_label(text);
                self.chip.set_visible(true);
            }
            None => self.chip.set_visible(false),
        }
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
        self.list.connect_row_activated(move |_, row| {
            let Some(page) = page.upgrade() else { return };
            let target = usize::try_from(row.index())
                .ok()
                .and_then(|i| page.state.borrow().rows.get(i).cloned());
            match target {
                Some(RowTarget::Open(document)) => page.open_document(&document),
                Some(RowTarget::Expand(module)) => {
                    {
                        let mut state = page.state.borrow_mut();
                        if !state.expanded.remove(module) {
                            state.expanded.insert(module);
                        }
                    }
                    page.fill_sidebar();
                }
                Some(RowTarget::Found(location)) => {
                    page.search.set_text("");
                    page.go(&location);
                }
                Some(RowTarget::Nothing) | None => {}
            }
        });
        let page = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            if let Some(page) = page.upgrade() {
                page.fill_sidebar();
            }
        });
        let page = Rc::downgrade(self);
        self.search.connect_activate(move |_| {
            let Some(page) = page.upgrade() else { return };
            let first = page
                .state
                .borrow()
                .rows
                .iter()
                .position(|r| matches!(r, RowTarget::Found(_) | RowTarget::Open(_)));
            if let Some(row) = first
                .and_then(|i| i32::try_from(i).ok())
                .and_then(|i| page.list.row_at_index(i))
            {
                row.activate();
            }
        });
        let page = Rc::downgrade(self);
        self.mode_toggle.connect_active_name_notify(move |toggle| {
            let Some(page) = page.upgrade() else { return };
            let mode = match toggle.active_name().as_deref() {
                Some("text") => Mode::Text,
                _ => Mode::Both,
            };
            page.state.borrow_mut().mode = mode;
            page.view_slot.set_visible(mode == Mode::Both);
            let callback = page.callbacks.borrow().on_mode.clone();
            if let Some(callback) = callback {
                callback(mode);
            }
        });
        let page = Rc::downgrade(self);
        uses_close.connect_clicked(move |_| {
            if let Some(page) = page.upgrade() {
                page.uses.set_reveal_child(false);
                page.state.borrow_mut().listed = None;
            }
        });
        // The 3D view takes what the editor doesn't need at first: about
        // 460 pixels, as in the design, once the split has its width.
        let page = Rc::downgrade(self);
        self.paned.add_tick_callback(move |paned, _| {
            let Some(page) = page.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let width = paned.width();
            if page.placed.get() {
                return glib::ControlFlow::Break;
            }
            if width > 1 && paned.is_mapped() {
                page.placed.set(true);
                let along = if paned.orientation() == gtk::Orientation::Horizontal {
                    width.saturating_sub(460).max(width / 2)
                } else {
                    paned.height() / 2
                };
                paned.set_position(along);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        self.connect_editor();
    }

    fn connect_editor(self: &Rc<Self>) {
        let view = &self.editor.view;
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
                gtk::gdk::Key::Escape if page.editor.has_peek() => page.editor.set_peek(None),
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
                page.hide_hover();
            }
        });
        view.add_controller(motion);

        // Scrolling and resizing move the overlays with the text.
        let page = Rc::downgrade(self);
        self.editor.root.vadjustment().connect_changed(move |_| {
            if let Some(page) = page.upgrade() {
                page.editor.place_overlays();
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
        let tab_label = label(&name, &[]);
        let button = gtk::Button::builder()
            .child(&tab_label)
            .css_classes(["stave-tab"])
            .tooltip_text(document.display(&self.state.borrow().folder))
            .build();
        let page = Rc::downgrade(self);
        let key = document.clone();
        button.connect_clicked(move |_| {
            if let Some(page) = page.upgrade() {
                page.open_document(&key);
            }
        });
        self.tab_row.append(&button);
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
        });
        Some(state.tabs.len().saturating_sub(1))
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
            self.hide_hover();
            self.state.borrow_mut().quiet = true;
            self.editor.show_buffer(&buffer, !read_only);
            self.state.borrow_mut().quiet = false;
            self.refresh_markers();
            self.fill_sidebar();
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
        {
            let mut state = self.state.borrow_mut();
            if let Some(tab) = state.tabs.iter_mut().find(|t| t.document == *document) {
                tab.lines = LineStarts::new(&text);
            }
            if state.buffers.edit(document, &text).is_err() {
                return;
            }
        }
        self.editor.set_peek(None);
        self.hide_hover();
        self.refresh_markers();
        self.schedule_analysis();
        self.schedule_check();
        let page = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(page) = page.upgrade() {
                page.editor.place_overlays();
            }
        });
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
            page.fill_sidebar_if_new();
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
            let mut problems: Vec<(Run, Severity)> = analysis
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
            self.show_lenses(analysis, *file);
        }
        self.cursor_moved_now(false);
    }

    fn check_problems(&self, document: &Document) -> Vec<Problem> {
        self.state
            .borrow()
            .problems
            .iter()
            .filter(|p| p.document == *document)
            .cloned()
            .collect()
    }

    fn show_lenses(self: &Rc<Self>, analysis: &Arc<Analysis>, file: stave_lang::FileId) {
        let mut lenses = Vec::new();
        for lens in analysis.lenses(file) {
            let Ok(line) = i32::try_from(lens.location.line.saturating_sub(1)) else {
                continue;
            };
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(4)
                .css_classes(["stave-lens"])
                .build();
            let uses = gtk::Button::builder()
                .label(lens.uses_text())
                .css_classes(["flat", "stave-lens-link"])
                .tooltip_text(format!("List the uses of {} (Shift+F12)", lens.name))
                .build();
            let page = Rc::downgrade(self);
            let symbol = lens.symbol;
            uses.connect_clicked(move |_| {
                if let Some(page) = page.upgrade() {
                    page.list_uses(symbol);
                }
            });
            row.append(&uses);
            if lens.shows_in_3d() {
                row.append(&label("·", &["stave-lens-dot"]));
                let show = gtk::Button::builder()
                    .label("show in 3D")
                    .css_classes(["flat", "stave-lens-link"])
                    .tooltip_text(format!("Select {} in the 3D view", lens.name))
                    .build();
                let page = Rc::downgrade(self);
                let analysis = analysis.clone();
                show.connect_clicked(move |_| {
                    let Some(page) = page.upgrade() else { return };
                    let callback = page.callbacks.borrow().on_show_in_3d.clone();
                    if let Some(callback) = callback {
                        callback(analysis.clone(), symbol);
                    }
                });
                row.append(&show);
            }
            lenses.push((row.upcast::<gtk::Widget>(), line));
        }
        self.editor.set_lenses(lenses);
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
        self.editor.mark_cursor_line(&buffer);
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
                let found = page.fresh().and_then(|(analysis, file)| {
                    let symbol = analysis.symbol_at(FilePosition {
                        file,
                        offset: page.cursor(),
                    })?;
                    Some((analysis, symbol))
                });
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
            self.crumbs.append(&crumb);
        }
    }

    /// Marks unsaved tabs and files.
    fn refresh_markers(&self) {
        let state = self.state.borrow();
        for (index, tab) in state.tabs.iter().enumerate() {
            let dirty = state.buffers.is_dirty(&tab.document);
            let name = tab.document.file_name();
            tab.label
                .set_label(&if dirty { format!("{name} •") } else { name });
            if Some(index) == state.active {
                tab.button.add_css_class("active");
            } else {
                tab.button.remove_css_class("active");
            }
            tab.button.set_tooltip_text(Some(&if dirty {
                format!(
                    "{} — not saved (Ctrl+S saves)",
                    tab.document.display(&state.folder)
                )
            } else {
                tab.document.display(&state.folder)
            }));
        }
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
        let open = gtk::Button::builder()
            .label("Open")
            .css_classes(["flat", "stave-lens-link"])
            .tooltip_text("Open the definition (F12)")
            .build();
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
        if let Some(old) = self.state.borrow_mut().hover_timer.take() {
            old.remove();
        }
        let offset = self.offset_at(x, y);
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

    fn hide_hover(&self) {
        if let Some(card) = self.state.borrow_mut().hover.take() {
            self.editor.view.remove(&card);
        }
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
        let mut anchor: Option<u32> = None;
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
            anchor.get_or_insert(problem.span.start());
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
            anchor = Some(hover.span.start());
        }
        let Some(anchor) = anchor else {
            return;
        };
        let lines = self.active_lines();
        let iter = super::editor::iter_at(&buffer, &lines, anchor);
        let rect = self.editor.view.iter_location(&iter);
        self.editor.view.add_overlay(
            &card,
            rect.x(),
            rect.y().saturating_add(rect.height()).saturating_add(6),
        );
        self.state.borrow_mut().hover = Some(card.upcast());
    }

    // ---- the sidebar ----

    fn fill_sidebar_if_new(self: &Rc<Self>) {
        if self.state.borrow().rows.is_empty() {
            self.fill_sidebar();
        }
    }

    /// Lists the project's files by folder and the standard library, or
    /// what "Go to file or name" finds.
    fn fill_sidebar(self: &Rc<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let query = self.search.text().to_string();
        let (folder, analysis, active, expanded) = {
            let state = self.state.borrow();
            (
                state.folder.clone(),
                state.analysis.clone(),
                state
                    .active
                    .and_then(|i| state.tabs.get(i))
                    .map(|t| t.document.clone()),
                state.expanded.clone(),
            )
        };
        let mut rows: Vec<RowTarget> = Vec::new();
        let mut selected: Option<usize> = None;
        let heading = |text: &str, first: bool| {
            gtk::ListBoxRow::builder()
                .child(&label(
                    text,
                    if first {
                        &["stave-heading", "first"]
                    } else {
                        &["stave-heading"]
                    },
                ))
                .activatable(false)
                .selectable(false)
                .build()
        };
        if !query.trim().is_empty() {
            let found = analysis
                .as_ref()
                .map(|a| a.search(&query, SEARCH_LIMIT))
                .unwrap_or_default();
            if found.is_empty() {
                self.list.append(&heading("NOTHING FOUND", true));
                rows.push(RowTarget::Nothing);
            }
            for found in found {
                let entry = gtk::Box::builder()
                    .orientation(gtk::Orientation::Vertical)
                    .css_classes(["stave-file", "found"])
                    .build();
                let name = label(&found.label, &[]);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                entry.append(&name);
                let detail = label(&found.detail, &["stave-detail"]);
                detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
                entry.append(&detail);
                self.list
                    .append(&gtk::ListBoxRow::builder().child(&entry).build());
                rows.push(RowTarget::Found(found.location));
            }
            self.state.borrow_mut().rows = rows;
            return;
        }
        // The project's own files, by folder.
        self.list.append(&heading("PROJECT", true));
        rows.push(RowTarget::Nothing);
        let files = stave_ide::project_files(&folder);
        let dirty: Vec<Document> = self.state.borrow().buffers.dirty().cloned().collect();
        let mut last_folder: Option<PathBuf> = None;
        // Files in the folder itself first, then each subfolder.
        let mut ordered: Vec<&PathBuf> = files
            .iter()
            .filter(|f| f.parent() == Some(folder.as_path()))
            .collect();
        ordered.extend(
            files
                .iter()
                .filter(|f| f.parent() != Some(folder.as_path())),
        );
        for path in ordered {
            let parent = path.parent().map(Path::to_path_buf);
            let nested = parent.as_deref() != Some(folder.as_path());
            if nested && parent != last_folder {
                let shown = parent
                    .as_deref()
                    .and_then(|p| p.strip_prefix(&folder).ok())
                    .map_or_else(String::new, |p| format!("{}/", p.display()));
                let row = gtk::ListBoxRow::builder()
                    .child(&label(&shown, &["stave-folder"]))
                    .activatable(false)
                    .selectable(false)
                    .build();
                self.list.append(&row);
                rows.push(RowTarget::Nothing);
            }
            last_folder = parent;
            let document = Document::File(path.clone());
            let name = document.file_name();
            let unsaved = dirty.contains(&document);
            let entry = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .css_classes(if nested {
                    vec!["stave-file", "nested"]
                } else {
                    vec!["stave-file"]
                })
                .build();
            let name_label = label(&name, &[]);
            name_label.set_hexpand(true);
            name_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            entry.append(&name_label);
            if unsaved {
                entry.append(&label("•", &["stave-unsaved"]));
                entry.set_tooltip_text(Some("Not saved"));
            }
            if active.as_ref() == Some(&document) {
                selected = Some(rows.len());
            }
            self.list
                .append(&gtk::ListBoxRow::builder().child(&entry).build());
            rows.push(RowTarget::Open(document));
        }
        // The standard library, read only.
        self.list.append(&heading("STANDARD LIBRARY", false));
        rows.push(RowTarget::Nothing);
        for module in stave_modules::STD_MODULES {
            let open = expanded.contains(module);
            let entry = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(6)
                .css_classes(["stave-file", "standard"])
                .build();
            entry.append(&super::icons::PathIcon::new(
                if open {
                    super::icons::EXPANDED
                } else {
                    super::icons::COLLAPSED
                },
                10,
            ));
            let name = label(module, &[]);
            name.set_hexpand(true);
            entry.append(&name);
            if module == "joinery" {
                entry.append(&label("read only", &["stave-detail", "stave-read-only"]));
            }
            self.list.append(
                &gtk::ListBoxRow::builder()
                    .child(&entry)
                    .selectable(false)
                    .tooltip_text("Part of Stave: read only")
                    .build(),
            );
            rows.push(RowTarget::Expand(module));
            if open {
                for document in Document::std_documents()
                    .filter(|d| matches!(d, Document::Std { module: m, .. } if *m == module))
                {
                    let entry = gtk::Box::builder()
                        .css_classes(["stave-file", "nested", "standard"])
                        .build();
                    entry.append(&label(&document.file_name(), &[]));
                    if active.as_ref() == Some(&document) {
                        selected = Some(rows.len());
                    }
                    self.list
                        .append(&gtk::ListBoxRow::builder().child(&entry).build());
                    rows.push(RowTarget::Open(document));
                }
            }
        }
        self.state.borrow_mut().rows = rows;
        match selected
            .and_then(|i| i32::try_from(i).ok())
            .and_then(|i| self.list.row_at_index(i))
        {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
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
