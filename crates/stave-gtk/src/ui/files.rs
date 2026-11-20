//! The Code panel: the open files as tabs (one each with a dot for its
//! worst problem and a yellow ● while unsaved, shown in the strip of the
//! frame holding the panel, beside the Model: [`FilesOutput::Tabs`]),
//! then breadcrumbs, the editor, the uses panel and a footer; "No file
//! open" while none is. The panel is docked wherever people put it, so
//! files open there. Files are opened from the Files panel, with the
//! finder (Ctrl+K) or by following a link.
//!
//! Everything it knows about the text comes from `stave-ide`: the open
//! buffers ([`Buffers`]), and an [`Analysis`] built again on a worker
//! thread a moment after each change, from the buffers as typed. The
//! window decides what a name means in the design and owns the history:
//! this page tells it where the cursor is ([`FilesOutput::Link`]), where
//! to go ([`FilesOutput::Go`]) and when typing has paused
//! ([`FilesOutput::Check`]), and it is told a place to show.
//!
//! It is a relm4 component, [`FilesComponent`] (docs/relm4-migration.md),
//! with the editor ([`Editor`]) as part of its view: its model is the only
//! state, only its `update` changes it, and its widgets, timers and worker
//! thread only send [`FilesInput`] messages. Programmatic cursor moves
//! block the buffer's own cursor handler instead of a "quiet" flag.
//!
//! What the window must know at once is never asked of the model
//! ("Request and reply" in the migration document):
//!
//! - questions whose answer depends on the order of what came before
//!   (how many files are unsaved, did saving them all work) are requests
//!   carrying the window's own continuation, `Then`, answered by an
//!   output that hands it back ([`FilesInput::CountUnsaved`] /
//!   [`FilesOutput::UnsavedCounted`], [`FilesInput::SaveAll`] /
//!   [`FilesOutput::Saved`]);
//! - what the window reads synchronously (the unsaved text for a check or
//!   an edit, a file's buffer, the latest analysis) comes from immutable,
//!   reference-counted GObjects and values the component publishes
//!   ([`FilesOutput::Documents`], [`FilesOutput::Analysed`]), which the
//!   window keeps in [`FilesPage`]. A buffer's own `modified` flag, kept
//!   equal to "has unsaved changes" by the component, lets the window
//!   build the overlay from the live text without asking.

use stave_app::i18n::{fill_in, ntr, tr, trc};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::check::Checked;
use stave_app::format::Formatted;
use stave_app::problems::Marks;
use stave_app::save_failure::SaveFailure;
use stave_app::session::{LineNumber, SourceLocation};
use stave_ide::{
    Analysis, Buffers, Document, FilePosition, Location, Overlay, SymbolId, TokenClass,
};
use stave_lang::Severity;
use stave_lang::ast::Span;

use super::editor::{Editor, EscapeProbe, LensClick, LensLine, LineStarts, Run, SearchEvent};
use super::tab_strip::PageTab;

/// How long typing must pause before the names are worked out again.
const ANALYSE_DELAY: Duration = Duration::from_millis(150);
/// How long typing must pause before the design is checked again.
const CHECK_DELAY: Duration = Duration::from_millis(700);
/// How long the cursor must rest on a name before the 3D view follows.
const LINK_DELAY: Duration = Duration::from_millis(120);
/// How long the pointer must rest on a word before its card shows.
const HOVER_DELAY: Duration = Duration::from_millis(450);
/// How many frames the overlays follow the text's layout after a change.
const SETTLE_FRAMES: u32 = 30;

/// The tag that strikes old lines through in the text while an edit's
/// change to them is pending ([`FilesInput::MarkPending`]).
const PENDING_TAG: &str = "stave-pending-old";

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

/// What the cursor rests on, for the 3D view to follow.
#[derive(Clone, Debug)]
pub enum CursorName {
    /// A name that means this symbol.
    Name(Rc<Analysis>, SymbolId),
    /// A name that means nothing (a typo): the link lets go.
    Unresolved,
    /// Anything else (a keyword, a field name, a number, a space, or text
    /// not worked out yet): the link stays as it is.
    Other,
}

/// A file an edit's commit wrote through a buffer (see `edit_writer.rs`),
/// for the window to take straight after the commit.
#[derive(Debug)]
pub(super) struct Written {
    /// The file.
    pub(super) document: Document,
    /// The buffer the writer made for it, when it had no tab: the
    /// component adopts it as the file's tab.
    pub(super) created: Option<gtk::TextBuffer>,
    /// The editor is open: it shows the file the change went into.
    pub(super) show: bool,
}

/// What the files component is told.
#[derive(Debug)]
pub enum FilesInput<Then> {
    // ---- the window ----
    /// Puts the keyboard in the text.
    FocusText,
    /// Opens a project folder: closes every tab (the window has asked
    /// about unsaved changes first) and works the new folder's names out.
    SetFolder(PathBuf),
    /// Saves the document showing (Ctrl+S).
    Save,
    /// Lays the document showing out with `stave fmt`.
    Format,
    /// Saves every document with unsaved changes; answered with
    /// [`FilesOutput::Saved`], which hands `then` back.
    SaveAll {
        /// What waits on the saving, handed back with the answer.
        then: Option<Then>,
    },
    /// Counts the documents with unsaved changes; answered with
    /// [`FilesOutput::UnsavedCounted`], which hands `then` back.
    CountUnsaved {
        /// What waits on the count, handed back with the answer.
        then: Then,
    },
    /// Lets go of unsaved changes (the window is closing and people
    /// chose not to save them).
    ForgetUnsaved,
    /// Ctrl+F: the text's own search bar while the keyboard is in the
    /// text, else [`FilesOutput::NotInText`].
    Search,
    /// The latest check's marks, for the dots on the tabs.
    ShowMarks(Rc<Marks>),
    /// The last check's problems, for the underlines and the cards;
    /// documents changed on disk by something else are reloaded.
    ShowResult(Option<Rc<Checked>>),
    /// Shows a place, telling the window where the cursor rests.
    ShowLocation(SourceLocation),
    /// Shows a place quietly (a selection made in the 3D view).
    Reveal(SourceLocation),
    /// Screenshot wishes, granted once the names are in.
    Wish(Wish),
    /// Follows the light or dark style.
    Restyle {
        /// Dark or light.
        dark: bool,
    },
    /// Opens a document in a tab (or shows its tab) at its top.
    OpenDocument(Document),
    /// The tab strip's tab at this place among the open files was chosen.
    ShowTab(usize),
    /// The owner made a buffer for a file with no tab (an edit's commit
    /// wrote through it, or the owner reads or marks it at once): the
    /// buffer becomes the file's tab.
    Adopt {
        /// The file.
        document: Document,
        /// Its buffer, holding the commit's text.
        buffer: gtk::TextBuffer,
    },
    /// Strike these lines of a document's text through while an edit's
    /// change to them is pending: each `(first, end)` from its first line
    /// to the line after its last, counted from 0 (`end` past the text
    /// reaches its end).
    MarkPending {
        /// The file.
        document: Document,
        /// The ranges of lines.
        lines: Vec<(i32, i32)>,
    },
    /// Take the pending marks off a document's text.
    ClearPending(Document),
    /// The editor's search heard this (its bar, or its count): handed
    /// back to the editor.
    EditorSearch(SearchEvent),
    // ---- its own widgets, timers and worker ----
    /// People changed a document's text (or something wrote into it).
    TextChanged(Document),
    /// People moved the cursor.
    CursorMoved,
    /// The pause before working the names out again is over.
    AnalyseDue(u64),
    /// The names were worked out on the worker thread.
    Analysed {
        /// The folder they were worked out for.
        folder: PathBuf,
        /// Which analysis it is.
        generation: u64,
        /// The analysis, when the project could be read.
        analysis: Option<Box<Analysis>>,
    },
    /// The pause before checking again is over.
    CheckDue(u64),
    /// The cursor rested long enough to tell the window what it is on.
    LinkDue(u64),
    /// The pointer rested on a word long enough for its card.
    HoverDue {
        /// Which rest it is.
        ticket: u64,
        /// The word's offset, if it was on one.
        offset: Option<u32>,
    },
    /// The pointer moved over the text.
    PointerMoved {
        /// Across, in the view.
        x: f64,
        /// Down, in the view.
        y: f64,
    },
    /// The pointer left the text, or the text was put away.
    PointerLeft,
    /// Ctrl+click on the text at a point.
    FollowAt {
        /// Across, in the view.
        x: f64,
        /// Down, in the view.
        y: f64,
    },
    /// F12: follows the name at the cursor.
    Follow,
    /// Alt+F12: peeks at the name at the cursor.
    Peek,
    /// Shift+F12: lists the uses of the name at the cursor.
    ListUsesAtCursor,
    /// Ctrl+I: the card for the word at the cursor.
    ShowInfo,
    /// A lens's "2 uses" was clicked.
    ListUses(SymbolId),
    /// A lens's "show in 3D" was clicked.
    ShowIn3d(SymbolId),
    /// The uses panel's × was clicked.
    CloseUses,
    /// Escape in the text: closes what is open over it.
    Escape,
    /// A use in the uses panel was clicked.
    Go(Location),
    /// The peek's "Open" was clicked.
    OpenPeek(Location),
    /// The text scrolled or was given another size.
    Settle,
    /// A frame while the overlays settle.
    SettleTick(u64),
    /// People chose to try a failed save again.
    SaveAgain(Document),
    /// People chose to save a copy after a failed save.
    SaveCopy(Document),
    /// A copy was written, or couldn't be.
    CopyWritten {
        /// Where.
        path: PathBuf,
        /// What went wrong, if anything.
        failure: Option<SaveFailure>,
    },
    /// The part of a wish that waits for the text to be laid out.
    GrantWishLater(Wish),
}

/// What the files component tells the window.
#[derive(Debug)]
pub enum FilesOutput<Then> {
    /// The cursor rests on a name, a broken name, or something else.
    Link(CursorName),
    /// Go to a place, remembering where the cursor was (a history entry).
    Go {
        /// Where the cursor was.
        from: Option<SourceLocation>,
        /// Where it goes.
        to: SourceLocation,
    },
    /// Typing paused: check the design again, reading the open buffers.
    Check,
    /// A lens's "show in 3D" was clicked.
    ShowIn3d {
        /// The analysis the name is in.
        analysis: Rc<Analysis>,
        /// The name.
        symbol: SymbolId,
    },
    /// A tab was chosen: open the editor.
    Open,
    /// Something brief to tell people. Failed saves get a dialog instead.
    Notice(String),
    /// The answer to [`FilesInput::SaveAll`]: `ok` is `false` when a file
    /// couldn't be saved (a dialog has said which and what to do), and
    /// whatever waited on the saving must not go on.
    Saved {
        /// Every file was saved.
        ok: bool,
        /// What waited on it.
        then: Option<Then>,
    },
    /// The answer to [`FilesInput::CountUnsaved`].
    UnsavedCounted {
        /// How many documents have unsaved changes.
        count: usize,
        /// What waited on it.
        then: Then,
    },
    /// The open documents and their buffers, sent whenever a tab opens or
    /// they all close. The window keeps its own clones (GObjects are
    /// reference-counted), to read their text without asking.
    Documents(Vec<(Document, gtk::TextBuffer)>),
    /// A new analysis of the project arrived.
    Analysed(Rc<Analysis>),
    /// The files with unsaved changes, sent when they change.
    Unsaved(BTreeSet<PathBuf>),
    /// The open files as tabs for the strip of the frame holding the
    /// panel, sent when they change.
    Tabs(Vec<PageTab>),
    /// Ctrl+F with the keyboard elsewhere: the finder's turn.
    NotInText,
}

/// One open document.
struct Tab {
    document: Document,
    buffer: gtk::TextBuffer,
    /// Where each line starts, shared (not copied) with whatever reads it.
    lines: Rc<LineStarts>,
    /// Which edit of the page last changed this document's text (see
    /// [`FilesComponent::edits`]): the text is unchanged while it stays.
    revision: u64,
    /// The buffer's cursor handler, blocked while the page moves the
    /// cursor itself.
    cursor_handler: glib::SignalHandlerId,
}

/// Whether the analysis was built from a document's text, worked out once
/// per analysis and edit rather than by comparing the texts on every
/// cursor move.
#[derive(Debug, Clone, Copy)]
struct FreshMemo {
    /// [`FilesComponent::analysis_generation`] when it was worked out.
    analysis: u64,
    /// [`Tab::revision`] when it was worked out.
    revision: u64,
    /// The analysis's file for the text, when it read the same text.
    file: Option<stave_lang::FileId>,
}

/// A problem in the open file, borrowed from the diagnostic it is part of.
struct Problem<'a> {
    span: Span,
    severity: Severity,
    message: &'a str,
    help: Option<&'a str>,
}

impl<'a> Problem<'a> {
    fn new(span: Span, diagnostic: &'a stave_lang::Diagnostic) -> Self {
        Problem {
            span,
            severity: diagnostic.severity,
            message: &diagnostic.message,
            help: diagnostic.help.as_deref(),
        }
    }

    /// Whether the text at an offset is part of it (an empty span covers
    /// the character after it too, so it can be hovered).
    fn covers(&self, offset: u32) -> bool {
        self.span.start() <= offset
            && (offset < self.span.end()
                || (self.span.start() == self.span.end()
                    && offset <= self.span.end().saturating_add(1)))
    }
}

/// Every place the check's problems point at in one of its files: where
/// each is, and the places that explain it (the kind's rule in a standard
/// file).
fn check_labels(checked: &Checked, file: stave_lang::FileId) -> impl Iterator<Item = Problem<'_>> {
    checked
        .opened
        .diagnostics
        .iter()
        .flat_map(move |diagnostic| {
            diagnostic
                .labels
                .iter()
                .filter(move |label| label.file == file)
                .map(move |label| Problem::new(label.span, diagnostic))
        })
}

fn label(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(classes.to_vec())
        .build()
}

/// The text's right-click menu: the editor's own actions, after GTK's
/// Cut, Copy and Paste. Each acts at the cursor.
fn editor_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    menu.append(Some(&tr("Go to Where It Is Made")), Some("editor.follow"));
    menu.append(Some(&tr("Peek at Where It Is Made")), Some("editor.peek"));
    menu.append(Some(&tr("List Where It Is Used")), Some("editor.uses"));
    menu.append(Some(&tr("Show Card")), Some("editor.show-info"));
    menu
}

/// The analysis's file for a document, when it read the same text.
fn same_text(analysis: &Analysis, document: &Document, text: &str) -> Option<stave_lang::FileId> {
    let file = analysis.file(document)?;
    (analysis.text(file)? == text).then_some(file)
}

fn escape(text: &str) -> String {
    glib::markup_escape_text(text).to_string()
}

/// The whole text of a buffer.
fn buffer_text(buffer: &gtk::TextBuffer) -> glib::GString {
    buffer.text(&buffer.start_iter(), &buffer.end_iter(), true)
}

/// A tab's tooltip: where the file is, and whether it is saved.
fn tab_tooltip(place: String, dirty: bool) -> String {
    if dirty {
        fill_in(
            &tr("{file} — not saved (Ctrl+S saves)"),
            &[("file", &place)],
        )
    } else {
        place
    }
}

/// The Code panel's model: the open files, the analysis and the last
/// check, and the editor showing them.
pub struct FilesComponent<Then: fmt::Debug + 'static> {
    /// Its own sender: for timers, dialogs and the worker thread.
    sender: ComponentSender<Self>,
    /// The panel, for dialogs' parent.
    root: gtk::Box,
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
    folder: PathBuf,
    buffers: Buffers,
    analysis: Option<Rc<Analysis>>,
    generation: u64,
    /// The generation of the analysis in `analysis`.
    analysis_generation: u64,
    /// Counts the page's text changes and opened tabs: each gives its tab
    /// a new [`Tab::revision`].
    edits: u64,
    /// Whether the analysis read the text showing, for the last
    /// (analysis, revision) asked about.
    fresh_memo: Option<FreshMemo>,
    analysing: bool,
    analyse_again: bool,
    tabs: Vec<Tab>,
    active: Option<usize>,
    /// The last check, kept by the session too: shared, not copied.
    checked: Option<Rc<Checked>>,
    /// The check's file for each document it has a problem in. Its
    /// problems are underlined only while the document still reads as the
    /// check read it (their spans are offsets into that text).
    checked_files: BTreeMap<Document, stave_lang::FileId>,
    /// The exact place to put the cursor when the next place shown is
    /// this location.
    pending: Option<(SourceLocation, u32)>,
    /// Each pause counts on: a timer's message for an older one is
    /// ignored (so no timer is ever removed after it has fired).
    analyse_ticket: u64,
    check_ticket: u64,
    link_ticket: u64,
    hover_ticket: u64,
    settle_ticket: u64,
    /// The settling under way: its frame callback on the text, and how
    /// many frames it has placed the overlays on. Removed after
    /// [`SETTLE_FRAMES`], or when a newer settling replaces it.
    settling: Option<(gtk::TickCallbackId, u32)>,
    /// The text the hover card is about, while it shows: the pointer
    /// leaving it hides the card at once.
    hover_span: Option<Span>,
    /// The worst problem in each file, by its source path.
    marks: Rc<Marks>,
    wish: Option<Wish>,
    /// The symbol whose uses the panel lists.
    listed: Option<SymbolId>,
    /// The text changed: tell the window what the cursor rests on once
    /// the names are worked out again.
    relink: bool,
    /// The files with unsaved changes, as last told to the window.
    unsaved: BTreeSet<PathBuf>,
    _then: std::marker::PhantomData<fn() -> Then>,
}

/// What the files component is made with.
#[derive(Debug)]
pub struct FilesInit {
    /// The tag table every buffer shares: the owner keeps it too, so a
    /// buffer it makes (see `edit_writer.rs`) can be adopted as a tab.
    pub table: gtk::TextTagTable,
}

impl<Then: fmt::Debug + 'static> Component for FilesComponent<Then> {
    type Init = FilesInit;
    type Input = FilesInput<Then>;
    type Output = FilesOutput<Then>;
    type CommandOutput = ();
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> gtk::Box {
        gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["stave-editor-column"])
            .build()
    }

    fn init(
        init: FilesInit,
        root: gtk::Box,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let crumbs = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["stave-crumbs"])
            .build();

        // A lens's clicks come back as the component's own input; the
        // forwarding ends when the editor, and with it every sender of
        // the lens's channel, is dropped.
        let (lens, clicks) = relm4::channel();
        relm4::spawn_local(
            clicks.forward(sender.input_sender().clone(), |click| match click {
                LensClick::Uses(symbol) => FilesInput::ListUses(symbol),
                LensClick::Show(symbol) => FilesInput::ShowIn3d(symbol),
            }),
        );
        // So does what the editor's search hears (its bar's outputs and
        // its counts), handed back to the editor in this update.
        let (search_events, heard) = relm4::channel();
        relm4::spawn_local(heard.forward(sender.input_sender().clone(), FilesInput::EditorSearch));
        let editor = Editor::new(&init.table, lens, search_events);

        // The uses panel.
        let uses_title = label("", &["stave-uses-title"]);
        uses_title.set_hexpand(true);
        let uses_close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text(tr("Close the list of uses (Escape)"))
            .css_classes(["flat", "circular"])
            .build();
        uses_close.update_property(&[gtk::accessible::Property::Label(&tr(
            "Close the list of uses",
        ))]);
        let input = sender.input_sender().clone();
        uses_close.connect_clicked(move |_| input.emit(FilesInput::CloseUses));
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

        // The footer: the cursor's line, on the right. The editor's keys
        // (Ctrl+click or F12 follows a name, Alt+F12 peeks, Alt+Left goes
        // back) are in the shortcuts dialog, not spelt out here.
        let line = label("", &["stave-footer-line"]);
        line.set_hexpand(true);
        line.set_xalign(1.0);
        let footer = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(14)
            .css_classes(["stave-editor-footer"])
            .build();
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
            .title(tr("No file open"))
            .description(tr("Open a file from Files, or find one with Ctrl+K. Files open in this panel, wherever you dock it."))
            .vexpand(true)
            .css_classes(["compact", "stave-code-placeholder"])
            .build();
        // It reads whole in any frame the panel is docked in.
        super::empty_state::fit(&placeholder);
        let placeholder = placeholder.upcast::<gtk::Widget>();
        // The open files show as tabs in the strip of the frame holding
        // this panel ([`FilesOutput::Tabs`]), beside the Model.
        root.append(&placeholder);
        root.append(&body);

        connect_editor(&editor, &uses, &sender);

        let model = FilesComponent {
            sender,
            root,
            placeholder,
            body,
            crumbs,
            editor,
            uses,
            uses_title,
            uses_list,
            line,
            folder: PathBuf::new(),
            buffers: Buffers::new(),
            analysis: None,
            generation: 0,
            analysis_generation: 0,
            edits: 0,
            fresh_memo: None,
            analysing: false,
            analyse_again: false,
            tabs: Vec::new(),
            active: None,
            checked: None,
            checked_files: BTreeMap::new(),
            pending: None,
            analyse_ticket: 0,
            check_ticket: 0,
            link_ticket: 0,
            hover_ticket: 0,
            settle_ticket: 0,
            settling: None,
            hover_span: None,
            marks: Rc::default(),
            wish: None,
            listed: None,
            relink: false,
            unsaved: BTreeSet::new(),
            _then: std::marker::PhantomData,
        };
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &gtk::Box) {
        match message {
            FilesInput::FocusText => {
                self.editor.view.grab_focus();
            }
            FilesInput::SetFolder(folder) => self.set_folder(folder),
            FilesInput::Save => self.save(),
            FilesInput::Format => self.format(),
            FilesInput::SaveAll { then } => {
                let result = self.buffers.save_all();
                self.refresh_markers();
                let ok = match result {
                    Ok(()) => true,
                    Err(error) => {
                        self.save_failed(&error);
                        false
                    }
                };
                self.tell(FilesOutput::Saved { ok, then });
            }
            FilesInput::CountUnsaved { then } => {
                let count = self.buffers.dirty().count();
                self.tell(FilesOutput::UnsavedCounted { count, then });
            }
            FilesInput::ForgetUnsaved => {
                self.buffers = Buffers::new();
                self.fresh_memo = None;
                self.refresh_markers();
            }
            FilesInput::Search => {
                if self.editor.has_focus() {
                    self.editor.start_search();
                } else {
                    self.tell(FilesOutput::NotInText);
                }
            }
            FilesInput::ShowMarks(marks) => {
                self.marks = marks;
                self.refresh_markers();
            }
            FilesInput::ShowResult(checked) => self.show_result(checked),
            FilesInput::ShowLocation(location) => self.show_place(&location, true),
            FilesInput::Reveal(location) => self.show_place(&location, false),
            FilesInput::Wish(wish) => {
                self.wish = Some(wish);
                self.grant_wish();
            }
            FilesInput::Restyle { dark } => {
                self.editor.colour(dark);
                self.refresh_view();
            }
            FilesInput::OpenDocument(document) => self.open_document(&document),
            FilesInput::ShowTab(index) => {
                if let Some(document) = self.tabs.get(index).map(|tab| tab.document.clone()) {
                    self.open_document(&document);
                    self.tell(FilesOutput::Open);
                }
            }
            FilesInput::Adopt { document, buffer } => self.adopt(document, buffer),
            FilesInput::MarkPending { document, lines } => self.mark_pending(&document, &lines),
            FilesInput::ClearPending(document) => self.clear_pending(&document),
            FilesInput::EditorSearch(event) => self.editor.search_event(event),
            FilesInput::TextChanged(document) => self.text_changed(&document),
            FilesInput::CursorMoved => self.cursor_moved_now(true),
            FilesInput::AnalyseDue(ticket) => {
                if ticket == self.analyse_ticket {
                    self.analyse_now();
                }
            }
            FilesInput::Analysed {
                folder,
                generation,
                analysis,
            } => self.analysed(folder, generation, analysis),
            FilesInput::CheckDue(ticket) => {
                if ticket == self.check_ticket {
                    self.tell(FilesOutput::Check);
                }
            }
            FilesInput::LinkDue(ticket) => {
                if ticket == self.link_ticket {
                    self.link();
                }
            }
            FilesInput::HoverDue { ticket, offset } => {
                if ticket == self.hover_ticket
                    && let Some(offset) = offset
                {
                    self.show_hover(offset);
                }
            }
            FilesInput::PointerMoved { x, y } => self.pointer_moved(x, y),
            FilesInput::PointerLeft => {
                self.cancel_hover_timer();
                self.hide_hover();
            }
            FilesInput::FollowAt { x, y } => {
                if let Some(offset) = self.offset_at(x, y) {
                    self.follow(offset);
                }
            }
            FilesInput::Follow => self.follow(self.cursor()),
            FilesInput::Peek => self.peek_at_cursor(),
            FilesInput::ListUsesAtCursor => self.list_uses_at_cursor(),
            FilesInput::ShowInfo => {
                self.cancel_hover_timer();
                self.show_hover(self.cursor());
            }
            FilesInput::ListUses(symbol) => self.list_uses(symbol),
            FilesInput::ShowIn3d(symbol) => {
                if let Some(analysis) = self.analysis.clone() {
                    self.tell(FilesOutput::ShowIn3d { analysis, symbol });
                }
            }
            FilesInput::CloseUses => {
                self.uses.set_reveal_child(false);
                self.listed = None;
            }
            FilesInput::Escape => {
                if self.editor.has_card() {
                    self.editor.hide_card();
                } else if self.editor.has_peek() {
                    self.editor.set_peek(None);
                } else if self.editor.has_search() {
                    self.editor.close_search();
                } else if self.uses.reveals_child() {
                    self.uses.set_reveal_child(false);
                }
            }
            FilesInput::Go(location) => self.go(&location),
            FilesInput::OpenPeek(location) => {
                self.editor.set_peek(None);
                self.go(&location);
            }
            FilesInput::Settle => self.settle_overlays(),
            FilesInput::SettleTick(ticket) => {
                if ticket == self.settle_ticket {
                    self.editor.place_overlays();
                    self.settled_a_frame();
                }
            }
            FilesInput::SaveAgain(document) => self.save_again(&document),
            FilesInput::SaveCopy(document) => self.save_copy(&document),
            FilesInput::CopyWritten { path, failure } => match failure {
                None => self.notice(fill_in(
                    &tr("Saved a copy as {file}; the open file still has unsaved changes"),
                    &[("file", &path.display())],
                )),
                Some(failure) => {
                    let dialog = adw::AlertDialog::builder()
                        .heading(&failure.heading)
                        .body(&failure.body)
                        .close_response("close")
                        .build();
                    dialog.add_response("close", &tr("_Close"));
                    dialog.present(Some(&self.root));
                }
            },
            FilesInput::GrantWishLater(wish) => self.grant_wish_later(&wish),
        }
    }
}

/// Connects the editor's widgets: each handler reads what its event gives
/// it (and, for Escape and Ctrl+click, whether the editor's own widgets
/// can take the event, which GTK must be told at once) and sends one
/// message.
fn connect_editor<Then: fmt::Debug + 'static>(
    editor: &Editor,
    uses: &gtk::Revealer,
    sender: &ComponentSender<FilesComponent<Then>>,
) {
    let view = &editor.view;
    // The editor's actions (F12, Alt+F12, Shift+F12, Ctrl+I), also in
    // the text's right-click menu.
    let actions = gio::SimpleActionGroup::new();
    type Message<Then> = fn() -> FilesInput<Then>;
    let editor_actions: [(&str, Message<Then>); 4] = [
        ("follow", || FilesInput::Follow),
        ("peek", || FilesInput::Peek),
        ("uses", || FilesInput::ListUsesAtCursor),
        ("show-info", || FilesInput::ShowInfo),
    ];
    for (name, message) in editor_actions {
        let action = gio::SimpleAction::new(name, None);
        let input = sender.input_sender().clone();
        action.connect_activate(move |_, _| input.emit(message()));
        actions.add_action(&action);
    }
    view.insert_action_group("editor", Some(&actions));
    let shortcuts = super::app::shortcut_controller(super::app::KeyScope::Editor);
    shortcuts.set_propagation_phase(gtk::PropagationPhase::Capture);
    view.add_controller(shortcuts);
    view.set_extra_menu(Some(&editor_menu()));

    // Escape closes what is open over the text: taken only when something
    // is (read from the widgets themselves), so it goes on otherwise.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let probe: EscapeProbe = editor.escape_probe();
    let uses = uses.downgrade();
    let input = sender.input_sender().clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        let open = probe.anything_open() || uses.upgrade().is_some_and(|uses| uses.reveals_child());
        if key == gtk::gdk::Key::Escape && open {
            input.emit(FilesInput::Escape);
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    view.add_controller(keys);

    // Ctrl+click follows the name under the pointer: claimed when there
    // is text there.
    let click = gtk::GestureClick::builder()
        .button(gtk::gdk::BUTTON_PRIMARY)
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let input = sender.input_sender().clone();
    click.connect_pressed(move |gesture, _, x, y| {
        let control = gesture
            .current_event_state()
            .contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let on_text = gesture
            .widget()
            .and_downcast::<gtk::TextView>()
            .is_some_and(|view| {
                let (bx, by) = view.window_to_buffer_coords(
                    gtk::TextWindowType::Widget,
                    crate::convert::int_of(x),
                    crate::convert::int_of(y),
                );
                view.iter_at_location(bx, by).is_some()
            });
        if control && on_text {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            input.emit(FilesInput::FollowAt { x, y });
        }
    });
    view.add_controller(click);

    // The card for the word under the pointer, after a short rest.
    let motion = gtk::EventControllerMotion::new();
    let input = sender.input_sender().clone();
    motion.connect_motion(move |_, x, y| input.emit(FilesInput::PointerMoved { x, y }));
    let input = sender.input_sender().clone();
    motion.connect_leave(move |_| input.emit(FilesInput::PointerLeft));
    view.add_controller(motion);
    // Closing the editor (or hiding the window) takes the card too.
    let input = sender.input_sender().clone();
    view.connect_unmap(move |_| input.emit(FilesInput::PointerLeft));

    // Scrolling and resizing move the overlays with the text.
    let input = sender.input_sender().clone();
    editor
        .vadjustment()
        .connect_changed(move |_| input.emit(FilesInput::Settle));
}

impl<Then: fmt::Debug + 'static> FilesComponent<Then> {
    /// Tells the window; nobody listening is not a mistake.
    fn tell(&self, output: FilesOutput<Then>) {
        let _ = self.sender.output(output);
    }

    fn notice(&self, text: String) {
        self.tell(FilesOutput::Notice(text));
    }

    /// Sends `message` to itself after `delay`. Only fails once the
    /// component has gone, when nothing needs doing.
    fn later(&self, delay: Duration, message: FilesInput<Then>) {
        let input = self.sender.input_sender().clone();
        glib::timeout_add_local_once(delay, move || {
            let _ = input.send(message);
        });
    }

    /// Opens a project folder: closes every tab and works the names out.
    fn set_folder(&mut self, folder: PathBuf) {
        self.folder = folder;
        self.buffers = Buffers::new();
        self.analysis = None;
        self.fresh_memo = None;
        self.tabs.clear();
        self.active = None;
        self.checked = None;
        self.checked_files.clear();
        self.listed = None;
        let empty = self.editor.new_buffer("");
        self.editor.show_buffer(&empty, false);
        self.uses.set_reveal_child(false);
        self.show_body(false);
        self.show_crumbs(&[]);
        self.analyse_now();
        self.publish_documents();
        self.refresh_markers();
    }

    /// The document showing, saved.
    fn save(&mut self) {
        let Some(document) = self.active_document() else {
            return;
        };
        if document.is_read_only() {
            let shown = document.display(&self.folder);
            self.notice(fill_in(
                &tr("{file} is part of the standard library and can't be changed"),
                &[("file", &shown)],
            ));
            return;
        }
        let result = self.buffers.save(&document);
        self.refresh_markers();
        if let Err(error) = result {
            self.save_failed(&error);
        }
    }

    /// A save failed: says so in a dialog that stays until it is
    /// answered, naming the file and what to do, with a way to try again
    /// or to save a copy somewhere else. The changes stay in the editor.
    fn save_failed(&self, error: &stave_ide::SaveError) {
        let failure = stave_app::save_failure::describe(error, &self.folder);
        // The document whose save failed, to try again or copy.
        let document = match error {
            stave_ide::SaveError::Unwritable { path, .. } => Some(Document::File(path.clone())),
            stave_ide::SaveError::ReadOnly(_) | stave_ide::SaveError::NotOpen(_) => None,
        };
        let dialog = adw::AlertDialog::builder()
            .heading(&failure.heading)
            .body(&failure.body)
            .close_response("cancel")
            .build();
        dialog.add_response("cancel", &tr("_Cancel"));
        if let Some(document) = document {
            dialog.add_responses(&[
                ("retry", tr("_Try Again").as_str()),
                ("copy", tr("Save a _Copy…").as_str()),
            ]);
            dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("copy"));
            let input = self.sender.input_sender().clone();
            dialog.connect_response(None, move |_, response| match response {
                "retry" => input.emit(FilesInput::SaveAgain(document.clone())),
                "copy" => input.emit(FilesInput::SaveCopy(document.clone())),
                _ => {}
            });
        }
        dialog.present(Some(&self.root));
    }

    /// Tries a failed save again; a second failure says so again.
    fn save_again(&mut self, document: &Document) {
        let result = self.buffers.save(document);
        self.refresh_markers();
        match result {
            Ok(()) => self.notice(fill_in(
                &tr("Saved {file}"),
                &[("file", &document.file_name())],
            )),
            Err(error) => self.save_failed(&error),
        }
    }

    /// Asks where, then writes a copy of a document's text there. The
    /// document itself stays open, unsaved, where it was.
    fn save_copy(&self, document: &Document) {
        let Some(text) = self.buffers.text(document).map(str::to_owned) else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title(tr("Save a Copy"))
            .initial_name(document.file_name())
            .modal(true)
            .build();
        let parent = self.root.root().and_downcast::<gtk::Window>();
        let input = self.sender.input_sender().clone();
        dialog.save(parent.as_ref(), gio::Cancellable::NONE, move |result| {
            let Ok(file) = result else { return };
            let Some(path) = file.path() else { return };
            let failure = stave_app::save_failure::save_copy(&path, &text).err();
            let _ = input.send(FilesInput::CopyWritten { path, failure });
        });
    }

    /// Lays the document showing out with `stave fmt`, as one step that
    /// Undo takes back. Never done unasked.
    fn format(&mut self) {
        let Some((document, buffer)) = self.active_tab() else {
            return;
        };
        if document.is_read_only() {
            return;
        }
        let text = buffer_text(&buffer);
        match stave_app::format::format_text(&document.source_path(), &text) {
            Ok(Formatted::Changed(formatted)) => {
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
            Ok(Formatted::Unchanged) => self.notice(tr("The file is already laid out")),
            Err(error) => self.notice(error.to_string()),
        }
    }

    /// Takes the last check's problems for the underlines and the cards,
    /// and reloads documents changed on disk by something else.
    fn show_result(&mut self, checked: Option<Rc<Checked>>) {
        let reloaded = self.reload_from_disk();
        self.checked_files.clear();
        if let Some(checked) = &checked {
            // Every file a problem points at, each underlined when it is
            // open (see `check_labels`).
            for diagnostic in &checked.opened.diagnostics {
                for label in &diagnostic.labels {
                    if let Some(source) = checked.opened.sources.get(label.file) {
                        self.checked_files
                            .entry(Document::from_source_path(source.path()))
                            .or_insert(label.file);
                    }
                }
            }
        }
        self.checked = checked;
        if reloaded {
            self.analyse_now();
        }
        self.refresh_view();
    }

    /// Keeps the lenses and the peek on their lines for the next half
    /// second, while GTK lays out the text (line heights below the view
    /// are only estimates until then). A newer settling replaces an older
    /// one, whose frames are then ignored.
    fn settle_overlays(&mut self) {
        self.editor.place_overlays();
        if let Some((older, _)) = self.settling.take() {
            older.remove();
        }
        self.settle_ticket = self.settle_ticket.wrapping_add(1);
        let ticket = self.settle_ticket;
        let input = self.sender.input_sender().clone();
        // The callback only sends; the update counts the frames and
        // removes it.
        let callback = self.editor.view.add_tick_callback(move |_, _| {
            if input.send(FilesInput::SettleTick(ticket)).is_err() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        self.settling = Some((callback, 0));
    }

    /// Counts a frame of the settling under way, and ends it after
    /// [`SETTLE_FRAMES`].
    fn settled_a_frame(&mut self) {
        let Some((callback, frames)) = self.settling.take() else {
            return;
        };
        let frames = frames.saturating_add(1);
        if frames >= SETTLE_FRAMES {
            callback.remove();
        } else {
            self.settling = Some((callback, frames));
        }
    }

    fn active_tab(&self) -> Option<(Document, gtk::TextBuffer)> {
        let tab = self.tabs.get(self.active?)?;
        Some((tab.document.clone(), tab.buffer.clone()))
    }

    fn active_document(&self) -> Option<Document> {
        Some(self.tabs.get(self.active?)?.document.clone())
    }

    /// The buffer of a document with a tab.
    fn buffer_of(&self, document: &Document) -> Option<gtk::TextBuffer> {
        self.tabs
            .iter()
            .find(|tab| tab.document == *document)
            .map(|tab| tab.buffer.clone())
    }

    /// Where the lines of the text showing start: shared, not copied.
    fn active_lines(&self) -> Rc<LineStarts> {
        self.active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| Rc::clone(&tab.lines))
            .unwrap_or_default()
    }

    fn offset_at(&self, x: f64, y: f64) -> Option<u32> {
        self.editor.offset_at(&self.active_lines(), x, y)
    }

    fn cursor(&self) -> u32 {
        self.editor.cursor(&self.active_lines())
    }

    /// Moves the cursor of the text showing without its cursor handler
    /// hearing (people didn't move it).
    fn quietly(&mut self, change: impl FnOnce(&mut Editor)) {
        match self.active.and_then(|index| self.tabs.get(index)) {
            Some(tab) => {
                tab.buffer.block_signal(&tab.cursor_handler);
                change(&mut self.editor);
                tab.buffer.unblock_signal(&tab.cursor_handler);
            }
            None => change(&mut self.editor),
        }
    }

    /// The analysis, when it was built from the text showing, with the
    /// file it calls that text. The texts are compared once per analysis
    /// and edit, not on every cursor move.
    fn fresh(&mut self) -> Option<(Rc<Analysis>, stave_lang::FileId)> {
        let analysis = self.analysis.clone()?;
        let tab = self.tabs.get(self.active?)?;
        let memo = self.fresh_memo.filter(|memo| {
            memo.analysis == self.analysis_generation && memo.revision == tab.revision
        });
        let file = match memo {
            Some(memo) => memo.file,
            None => {
                let file = self
                    .buffers
                    .text(&tab.document)
                    .and_then(|text| same_text(&analysis, &tab.document, text));
                self.fresh_memo = Some(FreshMemo {
                    analysis: self.analysis_generation,
                    revision: tab.revision,
                    file,
                });
                file
            }
        }?;
        Some((analysis, file))
    }

    /// Opens a document in a tab (or shows its tab) at its top.
    fn open_document(&mut self, document: &Document) {
        if let Some(index) = self.ensure_tab(document) {
            self.activate(index);
        }
    }

    fn ensure_tab(&mut self, document: &Document) -> Option<usize> {
        if let Some(index) = self.tabs.iter().position(|tab| tab.document == *document) {
            return Some(index);
        }
        let text = match self.buffers.open(document) {
            Ok(text) => text.to_owned(),
            Err(error) => {
                self.notice(error.to_string());
                return None;
            }
        };
        let buffer = self.editor.new_buffer(&text);
        let index = self.add_tab(document, buffer, &text);
        self.publish_documents();
        Some(index)
    }

    /// Adds a tab for an open document whose buffer holds `text`, with the
    /// buffer's handlers, which only send messages.
    fn add_tab(&mut self, document: &Document, buffer: gtk::TextBuffer, text: &str) -> usize {
        let input = self.sender.input_sender().clone();
        let key = document.clone();
        buffer.connect_changed(move |_| input.emit(FilesInput::TextChanged(key.clone())));
        let input = self.sender.input_sender().clone();
        let cursor_handler =
            buffer.connect_cursor_position_notify(move |_| input.emit(FilesInput::CursorMoved));
        buffer.set_modified(self.buffers.is_dirty(document));
        self.edits = self.edits.wrapping_add(1);
        self.tabs.push(Tab {
            document: document.clone(),
            lines: Rc::new(LineStarts::new(text)),
            revision: self.edits,
            buffer,
            cursor_handler,
        });
        self.tabs.len().saturating_sub(1)
    }

    /// The owner made a buffer for a file with no tab (an edit's commit
    /// wrote through it, or it is read or marked at once): the buffer
    /// becomes the file's tab, unsaved if its text differs from the file.
    /// Should the file have gained a tab meanwhile (the owner had not yet
    /// heard), a different text goes into that tab's buffer as one step
    /// that Undo takes back, and the owner hears the buffers again.
    fn adopt(&mut self, document: Document, buffer: gtk::TextBuffer) {
        let text = buffer_text(&buffer).to_string();
        if let Some(existing) = self.buffer_of(&document) {
            if buffer_text(&existing) != text {
                existing.begin_user_action();
                let (mut start, mut end) = existing.bounds();
                existing.delete(&mut start, &mut end);
                existing.insert(&mut start, &text);
                existing.end_user_action();
            }
            self.publish_documents();
            return;
        }
        if let Err(error) = self.buffers.open(&document) {
            self.notice(error.to_string());
            return;
        }
        // Not read-only: the writer writes only the project's own files.
        let _ = self.buffers.edit(&document, &text);
        self.add_tab(&document, buffer, &text);
        self.publish_documents();
        self.refresh_markers();
        if self.buffers.is_dirty(&document) {
            self.schedule_analysis();
        }
    }

    /// Strikes the pending old lines of a document's text through
    /// ([`FilesInput::MarkPending`]).
    fn mark_pending(&self, document: &Document, lines: &[(i32, i32)]) {
        let Some(buffer) = self.buffer_of(document) else {
            return;
        };
        let table = buffer.tag_table();
        let tag = table.lookup(PENDING_TAG).unwrap_or_else(|| {
            let tag = gtk::TextTag::builder()
                .name(PENDING_TAG)
                .strikethrough(true)
                .background_rgba(&gtk::gdk::RGBA::new(0.85, 0.2, 0.15, 0.16))
                .build();
            table.add(&tag);
            tag
        });
        for &(first, end) in lines {
            let Some(start) = buffer.iter_at_line(first) else {
                continue;
            };
            // Past the last line: to the end of the text.
            let end = buffer
                .iter_at_line(end)
                .unwrap_or_else(|| buffer.end_iter());
            buffer.apply_tag(&tag, &start, &end);
        }
    }

    /// Takes the pending marks off a document's text
    /// ([`FilesInput::ClearPending`]).
    fn clear_pending(&self, document: &Document) {
        if let Some(buffer) = self.buffer_of(document)
            && let Some(tag) = buffer.tag_table().lookup(PENDING_TAG)
        {
            let (start, end) = buffer.bounds();
            buffer.remove_tag(&tag, &start, &end);
        }
    }

    /// Tells the window the open documents and their buffers.
    fn publish_documents(&self) {
        let documents = self
            .tabs
            .iter()
            .map(|tab| (tab.document.clone(), tab.buffer.clone()))
            .collect();
        self.tell(FilesOutput::Documents(documents));
    }

    /// Shows the open file's text, or the placeholder when none is open.
    fn show_body(&self, open: bool) {
        self.body.set_visible(open);
        self.placeholder.set_visible(!open);
    }

    fn activate(&mut self, index: usize) {
        if self.active == Some(index) {
            return;
        }
        let Some((buffer, read_only)) = self
            .tabs
            .get(index)
            .map(|tab| (tab.buffer.clone(), tab.document.is_read_only()))
        else {
            return;
        };
        self.active = Some(index);
        self.show_body(true);
        self.hide_hover();
        self.editor.show_buffer(&buffer, !read_only);
        self.refresh_markers();
        self.refresh_view();
    }

    fn show_place(&mut self, location: &SourceLocation, report: bool) {
        let document = Document::from_source_path(&location.path);
        let Some(index) = self.ensure_tab(&document) else {
            return;
        };
        self.activate(index);
        let pending = self.pending.take();
        let lines = self.active_lines();
        let line = i32::try_from(location.line.get().saturating_sub(1)).unwrap_or(0);
        let cursor = self.editor.cursor(&lines);
        let offset = match pending {
            Some((wanted, offset)) if wanted == *location => offset,
            _ => {
                // Already on that line: leave the cursor where it is.
                let (current, _) = lines.position(cursor);
                if current == line {
                    cursor
                } else {
                    lines.offset(line, 0)
                }
            }
        };
        self.quietly(|editor| editor.go_to(&lines, offset));
        // What moving the cursor shows; the window hears only when asked.
        if self.editor.cursor(&lines) != cursor {
            self.cursor_moved_now(report);
        }
        if self.editor.view.is_mapped() {
            self.editor.view.grab_focus();
        }
    }

    fn text_changed(&mut self, document: &Document) {
        let Some(buffer) = self.buffer_of(document) else {
            return;
        };
        let text = buffer_text(&buffer);
        self.edits = self.edits.wrapping_add(1);
        let revision = self.edits;
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.document == *document) {
            tab.lines = Rc::new(LineStarts::new(&text));
            tab.revision = revision;
        }
        let was_dirty = self.buffers.is_dirty(document);
        if self.buffers.edit(document, &text).is_err() {
            return;
        }
        let dirty = self.buffers.is_dirty(document);
        // GTK marked it modified; it is so only while it differs from the
        // file (typing and undoing back is not a change).
        buffer.set_modified(dirty);
        self.editor.set_peek(None);
        self.hide_hover();
        if was_dirty != dirty {
            self.refresh_markers();
        }
        self.schedule_analysis();
        self.schedule_check();
        self.settle_overlays();
        self.relink = true;
    }

    fn schedule_analysis(&mut self) {
        self.analyse_ticket = self.analyse_ticket.wrapping_add(1);
        self.later(ANALYSE_DELAY, FilesInput::AnalyseDue(self.analyse_ticket));
    }

    fn schedule_check(&mut self) {
        self.check_ticket = self.check_ticket.wrapping_add(1);
        self.later(CHECK_DELAY, FilesInput::CheckDue(self.check_ticket));
    }

    /// Works the names out again from the buffers, on a worker thread.
    fn analyse_now(&mut self) {
        if self.analysing {
            self.analyse_again = true;
            return;
        }
        if self.folder.as_os_str().is_empty() {
            return;
        }
        self.analysing = true;
        self.generation = self.generation.wrapping_add(1);
        let folder = self.folder.clone();
        let overlay = self.buffers.overlay();
        let generation = self.generation;
        let input = self.sender.input_sender().clone();
        glib::spawn_future_local(async move {
            let loaded = gio::spawn_blocking(move || {
                let analysis = Analysis::load(&folder, &overlay).ok().map(Box::new);
                (folder, analysis)
            })
            .await;
            let (folder, analysis) = loaded.unwrap_or_default();
            // The page may have gone; then nobody needs the names.
            let _ = input.send(FilesInput::Analysed {
                folder,
                generation,
                analysis,
            });
        });
    }

    /// The names came back from the worker thread.
    fn analysed(&mut self, folder: PathBuf, generation: u64, analysis: Option<Box<Analysis>>) {
        self.analysing = false;
        if let Some(analysis) = analysis
            && folder == self.folder
            && generation == self.generation
        {
            let analysis = Rc::new(*analysis);
            self.analysis = Some(Rc::clone(&analysis));
            self.analysis_generation = generation;
            self.tell(FilesOutput::Analysed(analysis));
        }
        if std::mem::take(&mut self.analyse_again) {
            self.analyse_now();
        }
        self.refresh_view();
        self.grant_wish();
    }

    /// Repaints the document showing from the analysis and the problems:
    /// colours, underlines, lenses, breadcrumbs, the line.
    fn refresh_view(&mut self) {
        let Some((document, buffer)) = self.active_tab() else {
            return;
        };
        let lines = self.active_lines();
        let fresh = self.fresh();
        if let Some((analysis, file)) = &fresh {
            let runs: Vec<(Run, TokenClass)> = analysis
                .tokens(*file)
                .into_iter()
                .map(|token| {
                    (
                        Run {
                            start: token.span.start(),
                            end: token.span.end(),
                        },
                        token.class,
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
                    .filter_map(|diagnostic| {
                        let span = diagnostic.primary()?.span;
                        Some((
                            Run {
                                start: span.start(),
                                end: span.end(),
                            },
                            diagnostic.severity,
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        if let Some((checked, file)) = self.checked_file(&document) {
            problems.extend(check_labels(&checked, file).map(|problem| {
                (
                    Run {
                        start: problem.span.start(),
                        end: problem.span.end(),
                    },
                    problem.severity,
                )
            }));
        }
        self.editor.underline(&buffer, &lines, &problems);
        let relink = fresh.is_some() && std::mem::take(&mut self.relink);
        self.cursor_moved_now(relink);
    }

    /// The last check and its file for a document with problems, while
    /// the document still reads as the check read it (after an edit the
    /// problems' spans would point at the wrong text; the next check
    /// brings them back).
    fn checked_file(&self, document: &Document) -> Option<(Rc<Checked>, stave_lang::FileId)> {
        let checked = self.checked.as_ref()?;
        let file = *self.checked_files.get(document)?;
        let read = checked.opened.sources.get(file)?.text();
        (self.buffers.text(document)? == read).then(|| (Rc::clone(checked), file))
    }

    fn show_lenses(&mut self, analysis: &Rc<Analysis>, file: stave_lang::FileId) {
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

    /// The breadcrumbs, the line, the marked uses, and (after a rest,
    /// when `report`) the link to the 3D view follow the cursor.
    fn cursor_moved_now(&mut self, report: bool) {
        let Some((_, buffer)) = self.active_tab() else {
            return;
        };
        let cursor_iter = buffer.iter_at_mark(&buffer.get_insert());
        self.line.set_label(&fill_in(
            &tr("Line {line}"),
            &[("line", &cursor_iter.line().saturating_add(1))],
        ));
        let lines = self.active_lines();
        let offset = self.editor.cursor(&lines);
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        let position = FilePosition { file, offset };
        let crumbs: Vec<String> = analysis
            .breadcrumbs(position)
            .into_iter()
            .map(|crumb| crumb.label)
            .collect();
        self.show_crumbs(&crumbs);
        let symbol = analysis.symbol_at(position);
        let (declared, uses) = match symbol.and_then(|symbol| analysis.uses(symbol)) {
            Some(uses) => {
                let mut declared = None;
                let mut runs = Vec::new();
                for found in uses.uses.iter().filter(|found| found.location.file == file) {
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
        if report {
            self.link_ticket = self.link_ticket.wrapping_add(1);
            self.later(LINK_DELAY, FilesInput::LinkDue(self.link_ticket));
        }
    }

    /// Tells the window what the cursor rests on.
    fn link(&mut self) {
        let found = match self.fresh() {
            Some((analysis, file)) => {
                let position = FilePosition {
                    file,
                    offset: self.cursor(),
                };
                match analysis.symbol_at(position) {
                    Some(symbol) => CursorName::Name(analysis, symbol),
                    None if analysis.is_unresolved(position) => CursorName::Unresolved,
                    None => CursorName::Other,
                }
            }
            None => CursorName::Other,
        };
        self.tell(FilesOutput::Link(found));
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

    /// Tells the window the open files as tabs (name, worst problem,
    /// unsaved or not, the current one) and, when they changed, the files
    /// with unsaved changes; keeps each buffer's `modified` flag equal to
    /// "has unsaved changes".
    fn refresh_markers(&mut self) {
        let tabs: Vec<PageTab> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let dirty = self.buffers.is_dirty(&tab.document);
                tab.buffer.set_modified(dirty);
                PageTab {
                    title: tab.document.file_name(),
                    tooltip: tab_tooltip(tab.document.display(&self.folder), dirty),
                    unsaved: dirty,
                    mark: self.marks.file(&tab.document.source_path()),
                    active: Some(index) == self.active,
                }
            })
            .collect();
        self.tell(FilesOutput::Tabs(tabs));
        let unsaved: BTreeSet<PathBuf> = self
            .buffers
            .dirty()
            .filter_map(|document| match document {
                Document::File(path) => Some(path.clone()),
                Document::Std { .. } => None,
            })
            .collect();
        if unsaved != self.unsaved {
            self.unsaved = unsaved.clone();
            self.tell(FilesOutput::Unsaved(unsaved));
        }
    }

    // ---- navigation ----

    /// Goes to a place through the window, so Back returns here.
    fn go(&mut self, location: &Location) {
        let Some(line) = LineNumber::from_usize(location.line) else {
            return;
        };
        let to = SourceLocation {
            path: location.document.source_path(),
            line,
        };
        let from = self.active_document().and_then(|document| {
            let lines = self.active_lines();
            let (line, _) = lines.position(self.editor.cursor(&lines));
            Some(SourceLocation {
                path: document.source_path(),
                line: LineNumber::from_usize(usize::try_from(line).ok()?.saturating_add(1))?,
            })
        });
        self.pending = Some((to.clone(), location.span.start()));
        self.tell(FilesOutput::Go { from, to });
    }

    /// Follows the name at an offset to its declaration.
    fn follow(&mut self, offset: u32) {
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        match analysis.definition(FilePosition { file, offset }) {
            Some(location) => self.go(&location),
            None => self.notice(tr(
                "This word isn't declared anywhere: it is part of the language",
            )),
        }
    }

    fn list_uses_at_cursor(&mut self) {
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
    fn list_uses(&mut self, symbol: SymbolId) {
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
            if count == 0 {
                tr("no uses")
            } else {
                fill_in(
                    &ntr("{count} use", "{count} uses", count),
                    &[("count", &count)],
                )
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
            let input = self.sender.input_sender().clone();
            let location = found.location.clone();
            let click = gtk::GestureClick::new();
            click.connect_released(move |_, _, _, _| input.emit(FilesInput::Go(location.clone())));
            row.add_controller(click);
            self.uses_list.append(&row);
        }
        self.listed = Some(symbol);
        self.uses.set_reveal_child(true);
    }

    /// Opens a peek of the definition of the name under the cursor, under
    /// its line.
    fn peek_at_cursor(&mut self) {
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
            .and_then(|symbol| analysis.symbol(symbol))
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
        &self,
        analysis: &Rc<Analysis>,
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
                .label(trc("verb", "Open"))
                .css_classes(["flat", "stave-lens-link"])
                .tooltip_text(tr("Open the definition (F12)"))
                .build(),
        );
        let input = self.sender.input_sender().clone();
        let target = location.clone();
        open.connect_clicked(move |_| input.emit(FilesInput::OpenPeek(target.clone())));
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
            .map_or(location.line, |found| found.line);
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

    fn pointer_moved(&mut self, x: f64, y: f64) {
        self.cancel_hover_timer();
        let offset = self.offset_at(x, y);
        // Off the word the card is about (another word, past the end of a
        // line, a blank line): the card goes now, not after the rest.
        if let Some(span) = self.hover_span
            && !offset.is_some_and(|offset| span.start() <= offset && offset <= span.end())
        {
            self.hide_hover();
        }
        self.later(
            HOVER_DELAY,
            FilesInput::HoverDue {
                ticket: self.hover_ticket,
                offset,
            },
        );
    }

    /// The pointer's rest so far no longer counts.
    fn cancel_hover_timer(&mut self) {
        self.hover_ticket = self.hover_ticket.wrapping_add(1);
    }

    fn hide_hover(&mut self) {
        self.hover_span = None;
        self.editor.hide_card();
        self.editor
            .view
            .reset_property(gtk::AccessibleProperty::Description);
    }

    /// Shows the card for the word at an offset: its problem, or what it
    /// is with its doc comment and where it is declared.
    fn show_hover(&mut self, offset: u32) {
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
        // What the card says, for a screen reader: the text's description
        // while the card shows.
        let mut spoken: Vec<String> = Vec::new();
        let fast = analysis
            .diagnostics(file)
            .filter_map(|diagnostic| Some(Problem::new(diagnostic.primary()?.span, diagnostic)));
        let checked = self.checked_file(&document);
        let slow = checked
            .iter()
            .flat_map(|(checked, file)| check_labels(checked, *file));
        for problem in fast.chain(slow).filter(|problem| problem.covers(offset)) {
            let (word, class) = match problem.severity {
                Severity::Error => (tr("Error"), "error"),
                Severity::Warning => (tr("Warning"), "warning"),
                Severity::Note => (tr("Note"), "note"),
            };
            card.append(&label(&word, &["stave-hover-kind", class]));
            let message = label(problem.message, &["stave-hover-message"]);
            message.set_wrap(true);
            message.set_max_width_chars(48);
            card.append(&message);
            spoken.push(format!("{word}: {}", problem.message));
            if let Some(help) = problem.help {
                let help = fill_in(&tr("Help: {help}"), &[("help", &help)]);
                spoken.push(help.clone());
                let help = label(&help, &["stave-hover-help"]);
                help.set_wrap(true);
                help.set_max_width_chars(48);
                card.append(&help);
            }
            anchor = Some(anchor.map_or(problem.span, |span| span.to(problem.span)));
        }
        if anchor.is_none()
            && let Some(hover) = analysis.hover(FilePosition { file, offset })
        {
            card.append(&label(&hover.kind, &["stave-hover-kind", "stave-caps"]));
            let signature = label(&hover.signature, &["stave-hover-signature"]);
            signature.set_wrap(true);
            signature.set_max_width_chars(48);
            card.append(&signature);
            spoken.push(format!("{}: {}", hover.kind, hover.signature));
            if let Some(docs) = &hover.docs {
                spoken.push(docs.clone());
                let docs = label(docs, &["stave-hover-docs"]);
                docs.set_wrap(true);
                docs.set_max_width_chars(48);
                card.append(&docs);
            }
            if let Some(location) = &hover.location {
                card.append(&label(
                    &fill_in(
                        &tr("Defined in {place}"),
                        &[("place", &location.label(analysis.folder()))],
                    ),
                    &["stave-hover-where"],
                ));
            }
            anchor = Some(hover.span);
        }
        let Some(anchor) = anchor else {
            return;
        };
        self.hover_span = Some(anchor);
        self.editor
            .view
            .update_property(&[gtk::accessible::Property::Description(&spoken.join(". "))]);
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
    fn reload_from_disk(&mut self) -> bool {
        let documents: Vec<Document> = self.buffers.documents().cloned().collect();
        let mut changed = false;
        for document in documents {
            match self.buffers.reload(&document) {
                Ok(stave_ide::Reload::Reloaded) => {
                    let tab = self.tabs.iter().find(|tab| tab.document == document);
                    let text = self.buffers.text(&document);
                    if let (Some(tab), Some(text)) = (tab, text) {
                        let buffer = &tab.buffer;
                        let line = buffer.iter_at_mark(&buffer.get_insert()).line();
                        buffer.block_signal(&tab.cursor_handler);
                        buffer.set_text(text);
                        if let Some(iter) = buffer.iter_at_line(line) {
                            buffer.place_cursor(&iter);
                        }
                        buffer.unblock_signal(&tab.cursor_handler);
                        // The file's own text: nothing unsaved.
                        buffer.set_modified(false);
                    }
                    changed = true;
                }
                Ok(stave_ide::Reload::KeptChanges) => {
                    self.notice(fill_in(
                        &tr("{file} changed on disk; your unsaved changes are kept"),
                        &[("file", &document.file_name())],
                    ));
                }
                Ok(stave_ide::Reload::Unchanged) | Err(_) => {}
            }
        }
        changed
    }

    // ---- screenshots ----

    fn grant_wish(&mut self) {
        let Some((analysis, file)) = self.fresh() else {
            return;
        };
        let Some(wish) = self.wish.take() else {
            return;
        };
        if let Some(word) = &wish.cursor {
            let lines = self.active_lines();
            let (line, _) = lines.position(self.editor.cursor(&lines));
            let start = lines.offset(line, 0);
            if let Some(found) = analysis
                .text(file)
                .and_then(|text| text.get(usize::try_from(start).unwrap_or(0)..))
                .and_then(|rest| rest.lines().next()?.find(word.as_str()))
            {
                let offset = start.saturating_add(u32::try_from(found).unwrap_or(0));
                self.quietly(|editor| editor.go_to(&lines, offset));
                self.cursor_moved_now(true);
            }
        }
        // Once the text is laid out.
        self.later(Duration::from_millis(150), FilesInput::GrantWishLater(wish));
    }

    fn grant_wish_later(&mut self, wish: &Wish) {
        if let Some(text) = &wish.typing
            && let Some((_, buffer)) = self.active_tab()
        {
            // A key at a time, so undo groups it as it groups typing.
            let mut key = [0; 4];
            for character in text.chars() {
                buffer.insert_interactive_at_cursor(character.encode_utf8(&mut key), true);
            }
            // What was typed is worked out as typing is, from its own
            // queued messages; the rest of the wish acts on the text now.
            self.text_changed_now();
        }
        if wish.undo
            && let Some((_, buffer)) = self.active_tab()
        {
            buffer.undo();
            self.text_changed_now();
        }
        if let Some(text) = &wish.search {
            self.editor.find(text);
        }
        if wish.follow {
            self.follow(self.cursor());
        }
        if wish.uses {
            self.list_uses_at_cursor();
        }
        if wish.peek {
            self.peek_at_cursor();
        }
        if wish.hover {
            self.show_hover(self.cursor());
        }
    }

    /// Takes the text showing as it is now, as its queued change message
    /// would (that message then finds nothing new).
    fn text_changed_now(&mut self) {
        if let Some(document) = self.active_document() {
            self.text_changed(&document);
        }
    }
}

/// One line of source as Pango markup, coloured by the file's tokens.
fn markup_line(text: &str, start: u32, tokens: &[stave_ide::SemanticToken], dark: bool) -> String {
    let end = start.saturating_add(u32::try_from(text.len()).unwrap_or(0));
    let mut out = String::new();
    let mut at = 0usize;
    for token in tokens
        .iter()
        .filter(|token| token.span.start() >= start && token.span.end() <= end)
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

/// The Code panel as its owner holds it: the one owner of the component's
/// controller (its outputs forwarded at launch to where the owner said),
/// and the owner's own copies of what the component publishes, read
/// without asking it: the open documents' buffers (for the unsaved text a
/// check or an edit reads, and for the edit writer), the latest analysis
/// (handed over by the owner when [`FilesOutput::Analysed`] reaches it),
/// and the shared tag table.
///
/// Its senders are queued: they take effect when the main loop next
/// runs. `Then` is what the owner wants done once a request is answered.
pub struct FilesPage<Then: fmt::Debug + 'static> {
    /// The panel's content.
    pub root: gtk::Box,
    controller: Controller<FilesComponent<Then>>,
    /// The open documents' buffers, as last published, plus any an edit's
    /// commit made since (taken by [`FilesPage::take_written`]).
    buffers: BTreeMap<Document, gtk::TextBuffer>,
    /// The latest analysis, as last published.
    analysis: Option<Rc<Analysis>>,
    /// The tag table every buffer shares.
    table: gtk::TextTagTable,
    /// Where the edit writer says what it wrote, and where it is read.
    written: (mpsc::Sender<Written>, mpsc::Receiver<Written>),
}

impl<Then: fmt::Debug + 'static> FilesPage<Then> {
    /// An empty panel, each output made the owner's message by `map` and
    /// sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(FilesOutput<Then>) -> X + 'static,
    ) -> FilesPage<Then> {
        let table = gtk::TextTagTable::new();
        let controller = FilesComponent::<Then>::builder()
            .launch(FilesInit {
                table: table.clone(),
            })
            .forward(to, map);
        FilesPage {
            root: controller.widget().clone(),
            controller,
            buffers: BTreeMap::new(),
            analysis: None,
            table,
            written: mpsc::channel(),
        }
    }

    /// Queues a message for the component; once it has gone there is
    /// nobody to tell.
    pub fn send(&self, input: FilesInput<Then>) {
        let _ = self.controller.sender().send(input);
    }

    /// Takes a [`FilesOutput::Analysed`]: the latest analysis.
    pub fn set_analysis(&mut self, analysis: Rc<Analysis>) {
        self.analysis = Some(analysis);
    }

    /// Takes a [`FilesOutput::Documents`]: the open documents' buffers.
    pub fn set_documents(&mut self, documents: Vec<(Document, gtk::TextBuffer)>) {
        self.buffers = documents.into_iter().collect();
    }

    /// The latest analysis of the project.
    pub fn analysis(&self) -> Option<Rc<Analysis>> {
        self.analysis.clone()
    }

    /// The unsaved texts, read live from the buffers: what a check reads
    /// (which text it read comes back with its result, so its underlines
    /// go on the right text), and what an edit started in the window
    /// reads, over the disk.
    pub fn edit_files(&self) -> Overlay {
        let mut overlay = Overlay::new();
        for (document, buffer) in &self.buffers {
            if let Document::File(path) = document
                && buffer.is_modified()
            {
                overlay.insert(path, buffer_text(buffer).as_str());
            }
        }
        overlay
    }

    /// The unsaved texts, for a check that reads what the editor shows.
    pub fn overlay_for_check(&self) -> Overlay {
        self.edit_files()
    }

    /// The text buffer of a file of the project, when it has a tab.
    #[cfg_attr(not(test), allow(dead_code, reason = "tests read the buffers"))]
    pub fn open_buffer(&self, path: &Path) -> Option<gtk::TextBuffer> {
        self.buffers
            .get(&Document::File(path.to_path_buf()))
            .cloned()
    }

    /// The buffer of a document, made here (from its text on disk, or
    /// Stave's for a standard file) and handed to the component as its tab
    /// when it has none, so it can be read and marked at once, as when the
    /// page opened tabs synchronously. `None` when it can't be read: the
    /// component says so when it is shown.
    fn ensure_buffer(&mut self, document: &Document) -> Option<gtk::TextBuffer> {
        if let Some(buffer) = self.buffers.get(document) {
            return Some(buffer.clone());
        }
        let mut reader = Buffers::new();
        let text = reader.open(document).ok()?;
        let buffer = super::editor::new_buffer(&self.table, text);
        // The file's own text: nothing unsaved.
        buffer.set_modified(false);
        self.buffers.insert(document.clone(), buffer.clone());
        self.send(FilesInput::Adopt {
            document: document.clone(),
            buffer: buffer.clone(),
        });
        Some(buffer)
    }

    /// A writer for an edit's commit: through these buffers (opening a
    /// tab for a file written while the editor is open), else to disk.
    pub(super) fn writer(&self, editor_open: bool) -> super::edit_writer::BufferWriter {
        super::edit_writer::BufferWriter::through(
            self.buffers.clone(),
            editor_open,
            self.table.clone(),
            self.written.0.clone(),
        )
    }

    /// Takes what the edit writer wrote since last asked: a buffer it made
    /// is kept here at once (so the unsaved text is read straight after
    /// the commit) and adopted by the component as the file's tab; a file
    /// written while the editor is open is shown.
    pub fn take_written(&mut self) {
        while let Ok(written) = self.written.1.try_recv() {
            if let Some(buffer) = written.created {
                self.buffers
                    .insert(written.document.clone(), buffer.clone());
                self.send(FilesInput::Adopt {
                    document: written.document.clone(),
                    buffer,
                });
            }
            if written.show {
                self.send(FilesInput::OpenDocument(written.document));
            }
        }
    }

    /// Puts the keyboard in the text.
    pub fn focus_text(&self) {
        self.send(FilesInput::FocusText);
    }

    /// Opens a project folder (the window has asked about unsaved changes
    /// first).
    pub fn set_folder(&mut self, folder: &Path) {
        self.buffers.clear();
        self.send(FilesInput::SetFolder(folder.to_path_buf()));
    }

    /// Saves the document showing.
    pub fn save(&self) {
        self.send(FilesInput::Save);
    }

    /// Lays the document showing out with `stave fmt`.
    pub fn format(&self) {
        self.send(FilesInput::Format);
    }

    /// Saves every document with unsaved changes; the answer is
    /// [`FilesOutput::Saved`], with `then`.
    pub fn save_all(&self, then: Option<Then>) {
        self.send(FilesInput::SaveAll { then });
    }

    /// Counts the documents with unsaved changes; the answer is
    /// [`FilesOutput::UnsavedCounted`], with `then`.
    pub fn count_unsaved(&self, then: Then) {
        self.send(FilesInput::CountUnsaved { then });
    }

    /// Lets go of unsaved changes.
    pub fn forget_unsaved(&self) {
        self.send(FilesInput::ForgetUnsaved);
    }

    /// Ctrl+F: the text's search, or [`FilesOutput::NotInText`].
    pub fn search(&self) {
        self.send(FilesInput::Search);
    }

    /// Takes the latest check's marks for the dots on the tabs.
    pub fn show_marks(&self, marks: Rc<Marks>) {
        self.send(FilesInput::ShowMarks(marks));
    }

    /// Takes the last check's problems.
    pub fn show_result(&self, checked: Option<Rc<Checked>>) {
        self.send(FilesInput::ShowResult(checked));
    }

    /// Shows a place, telling the window where the cursor rests. Its
    /// file's buffer is in the window's copies at once (made here and
    /// adopted by the component when it has no tab), so an edit's commit
    /// straight after writes through it.
    pub fn show_location(&mut self, location: &SourceLocation) {
        self.ensure_buffer(&Document::from_source_path(&location.path));
        self.send(FilesInput::ShowLocation(location.clone()));
    }

    /// Shows a place quietly. Its file's buffer is in the window's copies
    /// at once, as with [`FilesPage::show_location`].
    pub fn reveal(&mut self, location: &SourceLocation) {
        self.ensure_buffer(&Document::from_source_path(&location.path));
        self.send(FilesInput::Reveal(location.clone()));
    }

    /// Asks for screenshot wishes once the names are in.
    pub fn wish(&self, wish: Wish) {
        self.send(FilesInput::Wish(wish));
    }

    /// Follows the light or dark style.
    pub fn restyle(&self, dark: bool) {
        self.send(FilesInput::Restyle { dark });
    }

    /// Opens a document in a tab (or shows its tab) at its top.
    #[cfg_attr(not(test), allow(dead_code, reason = "tests open files"))]
    pub fn open_document(&self, document: &Document) {
        self.send(FilesInput::OpenDocument(document.clone()));
    }

    /// The component's model, for tests.
    #[cfg(test)]
    pub(crate) fn model(&self) -> std::cell::Ref<'_, FilesComponent<Then>> {
        self.controller.model()
    }

    /// The component's input sender, for tests.
    #[cfg(test)]
    pub(crate) fn sender(&self) -> &relm4::Sender<FilesInput<Then>> {
        self.controller.sender()
    }
}

#[cfg(test)]
impl<Then: fmt::Debug + 'static> FilesComponent<Then> {
    /// How many documents have unsaved changes, for tests.
    pub(crate) fn unsaved_count(&self) -> usize {
        self.buffers.dirty().count()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    type Page = FilesPage<&'static str>;

    /// A page whose outputs are recorded, as its owner would hear them.
    fn recorded_page() -> (Page, Rc<RefCell<Vec<FilesOutput<&'static str>>>>) {
        let (to, heard) = crate::ui::gtk_test::recorder();
        (Page::new(&to, |output| output), heard)
    }

    /// A copy of an example project in a folder of its own, so saving
    /// never touches the examples.
    fn scratch_copy(example: &str, name: &str) -> PathBuf {
        let from = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(example);
        let to = std::env::temp_dir().join(format!("stave-files-test-{name}"));
        let _ = std::fs::remove_dir_all(&to);
        std::fs::create_dir_all(&to).unwrap();
        for entry in std::fs::read_dir(&from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
        to.canonicalize().unwrap()
    }

    #[test]
    fn a_dropped_page_is_freed() {
        on_gtk_thread(page_is_freed);
    }

    /// Every handler, timer and worker of the page holds only its sender:
    /// once the owner lets go of it, the component, its widgets and the
    /// owner's callback are freed, with a folder opened, its analysis
    /// running and a file open.
    fn page_is_freed() {
        let token = Rc::new(());
        let held = Rc::clone(&token);
        let to = crate::ui::gtk_test::listener(move |_: FilesOutput<&'static str>| {
            let _ = &held;
        });
        let mut page = Page::new(&to, |output| output);
        drop(to);
        let callback = Rc::downgrade(&token);
        drop(token);
        let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/side_table")
            .canonicalize()
            .unwrap_or_default();
        page.set_folder(&folder);
        page.open_document(&Document::File(folder.join("table.stave")));
        let root = page.root.downgrade();
        let input = page.sender().clone();
        settle();
        assert!(root.upgrade().is_some(), "the widget lives while held");
        drop(page);
        settle();
        assert!(root.upgrade().is_none(), "its widget is freed");
        assert!(
            input.send(FilesInput::FocusText).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn typing_is_published_for_the_owner_to_read_at_once() {
        on_gtk_thread(typing_is_published);
    }

    /// Opening a file publishes its buffer; typing into it marks it
    /// modified at once (GTK's own flag), so the owner's overlay has the
    /// new text before the component has heard; the component then tells
    /// the owner which files are unsaved and the tabs, and saving all
    /// answers with the owner's continuation and clears the flag.
    fn typing_is_published() {
        let folder = scratch_copy("wall_shelf", "typing");
        let path = folder.join("shelf.stave");
        let (mut page, heard) = recorded_page();
        page.set_folder(&folder);
        page.open_document(&Document::File(path.clone()));
        settle();
        let documents = heard
            .borrow_mut()
            .drain(..)
            .filter_map(|output| match output {
                FilesOutput::Documents(documents) => Some(documents),
                _ => None,
            })
            .next_back();
        page.set_documents(documents.unwrap_or_default());
        let Some(buffer) = page.open_buffer(&path) else {
            panic!("the file's buffer is published");
        };
        assert!(page.edit_files().is_empty(), "nothing unsaved yet");

        buffer.insert(&mut buffer.end_iter(), "\n// typed\n");
        // Before the component has run: read straight from the buffer.
        use stave_modules::FileSource;
        let overlay = page.edit_files();
        assert!(
            overlay
                .read(&path)
                .is_ok_and(|text| text.ends_with("// typed\n")),
            "the overlay has the typed text at once"
        );
        settle();
        let unsaved = heard.borrow().iter().find_map(|output| match output {
            FilesOutput::Unsaved(unsaved) => Some(unsaved.clone()),
            _ => None,
        });
        assert_eq!(unsaved, Some(BTreeSet::from([path.clone()])));
        assert!(
            heard
                .borrow()
                .iter()
                .any(|output| matches!(output, FilesOutput::Tabs(tabs) if tabs.iter().any(|tab| tab.unsaved))),
            "the tab is marked unsaved"
        );

        heard.borrow_mut().clear();
        page.save_all(Some("go on"));
        assert!(buffer.is_modified(), "saving is only queued");
        settle();
        assert!(!buffer.is_modified(), "saved: nothing unsaved");
        assert!(page.edit_files().is_empty());
        assert!(
            heard.borrow().iter().any(|output| matches!(
                output,
                FilesOutput::Saved {
                    ok: true,
                    then: Some("go on")
                }
            )),
            "the answer hands the continuation back"
        );
        assert!(
            std::fs::read_to_string(&path).is_ok_and(|text| text.ends_with("// typed\n")),
            "written to disk"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn requests_are_answered_in_order_after_what_came_before() {
        on_gtk_thread(requests_are_answered_in_order);
    }

    /// The count of unsaved files is asked by message: a change written
    /// into a buffer just before the question (its message queued first)
    /// is counted, and forgetting the changes before asking again counts
    /// none. Nothing is answered until the main loop runs.
    fn requests_are_answered_in_order() {
        let folder = scratch_copy("wall_shelf", "order");
        let path = folder.join("shelf.stave");
        let (mut page, heard) = recorded_page();
        page.set_folder(&folder);
        page.open_document(&Document::File(path.clone()));
        settle();
        let documents = heard
            .borrow_mut()
            .drain(..)
            .filter_map(|output| match output {
                FilesOutput::Documents(documents) => Some(documents),
                _ => None,
            })
            .next_back();
        page.set_documents(documents.unwrap_or_default());
        let Some(buffer) = page.open_buffer(&path) else {
            panic!("the file's buffer is published");
        };
        heard.borrow_mut().clear();

        buffer.insert(&mut buffer.end_iter(), "\n");
        page.count_unsaved("first");
        page.forget_unsaved();
        page.count_unsaved("second");
        assert!(heard.borrow().is_empty(), "only queued");
        settle();
        let counts: Vec<(usize, &str)> = heard
            .borrow()
            .iter()
            .filter_map(|output| match output {
                FilesOutput::UnsavedCounted { count, then } => Some((*count, *then)),
                _ => None,
            })
            .collect();
        assert_eq!(counts, [(1, "first"), (0, "second")]);
        assert!(!buffer.is_modified(), "forgotten changes are not unsaved");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_buffer_made_by_an_edit_is_adopted_as_a_tab() {
        on_gtk_thread(made_buffer_is_adopted);
    }

    /// The edit writer makes a buffer for a file with no tab: the owner
    /// keeps it at once, and the component adopts it as the file's tab,
    /// unsaved, and shows it.
    fn made_buffer_is_adopted() {
        let folder = scratch_copy("wall_shelf", "adopt");
        let path = folder.join("shelf.stave");
        let (mut page, heard) = recorded_page();
        page.set_folder(&folder);
        settle();
        let text = std::fs::read_to_string(&path).unwrap();
        let buffer = super::super::editor::new_buffer(&page.table, &text);
        buffer.insert(&mut buffer.end_iter(), "\n");
        let _ = page.written.0.send(Written {
            document: Document::File(path.clone()),
            created: Some(buffer.clone()),
            show: true,
        });
        page.take_written();
        assert_eq!(
            page.open_buffer(&path).as_ref(),
            Some(&buffer),
            "kept at once"
        );
        assert!(!page.edit_files().is_empty(), "its text is unsaved at once");
        settle();
        assert_eq!(page.model().unsaved_count(), 1, "adopted, unsaved");
        assert!(
            heard
                .borrow()
                .iter()
                .any(|output| matches!(output, FilesOutput::Tabs(tabs) if tabs.len() == 1 && tabs[0].active && tabs[0].unsaved)),
            "shown as the active tab"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_pending_lines_are_struck_by_the_component() {
        on_gtk_thread(pending_lines_are_struck);
    }

    /// The window used to apply the pending tag to the Code panel's
    /// buffer itself; now it asks, after revealing the file, and the
    /// component strikes the lines through in its own update and takes
    /// them off again when told, with no text changed.
    fn pending_lines_are_struck() {
        let folder = scratch_copy("wall_shelf", "pending");
        let path = folder.join("shelf.stave");
        let (mut page, heard) = recorded_page();
        page.set_folder(&folder);
        settle();
        page.reveal(&SourceLocation {
            path: path.to_string_lossy().into_owned(),
            line: stave_app::session::LineNumber::from_usize(1).unwrap(),
        });
        let document = Document::File(path.clone());
        page.send(FilesInput::MarkPending {
            document: document.clone(),
            lines: vec![(0, 1)],
        });
        let Some(buffer) = page.open_buffer(&path) else {
            panic!("revealing made the file's buffer");
        };
        let struck = |buffer: &gtk::TextBuffer| {
            buffer
                .tag_table()
                .lookup(PENDING_TAG)
                .is_some_and(|tag| buffer.start_iter().has_tag(&tag))
        };
        assert!(!struck(&buffer), "only queued");
        heard.borrow_mut().clear();
        settle();
        assert!(struck(&buffer), "the first line is struck through");
        let second = buffer.iter_at_line(1).unwrap();
        assert!(
            !buffer
                .tag_table()
                .lookup(PENDING_TAG)
                .is_some_and(|tag| second.has_tag(&tag)),
            "only the lines asked for"
        );
        assert!(!buffer.is_modified(), "marking changes no text");
        page.send(FilesInput::ClearPending(document));
        settle();
        assert!(!struck(&buffer), "taken off again");
        assert!(
            !heard
                .borrow()
                .iter()
                .any(|output| matches!(output, FilesOutput::Unsaved(_))),
            "nothing became unsaved"
        );
        let _ = std::fs::remove_dir_all(&folder);
    }
}
