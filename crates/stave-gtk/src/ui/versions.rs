//! The Versions panel and what the window does with the project's
//! versions (`docs/gui.md`, "Versions").
//!
//! - **The panel** ([`VersionsPanelComponent`]): Suggestions from AI apps
//!   above the project's versions, newest first. Each suggestion has Show
//!   (tried on the design and shown as a ghost, as any change in progress),
//!   Apply and Discard; Undo undoes the last change by anyone. While
//!   versions are off it offers to turn them on; for a project inside
//!   another project's history it says why they can't be.
//! - **Saving merges**: every save the Code panel makes
//!   ([`files::FilesOutput::SavedFiles`]) is recorded and merged into the
//!   project's main line; the first save after opening a project whose
//!   versions are off asks "Turn on versions for this project?" (Turn On /
//!   Not Now, the latter remembered per project).
//! - **Catching up**: when someone else changes the project (an AI app
//!   applies a suggestion), the window hears it from the versions' folder
//!   and writes the changed files at once, unless it holds one of them
//!   unsaved; the Code panel then reloads its clean files.
//!
//! The decisions are `stave_app::versions`; the work runs off the main
//! thread and comes back as a [`VersionsInput`]. The window never names
//! the revision system behind them.

use stave_app::i18n::{fill_in, tr};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk::glib;
use relm4::{Component, ComponentController, ComponentParts, ComponentSender, Controller};
use stave_app::editing::EditCommand;
use stave_app::session::Command;
use stave_app::versions::{
    self as history, Actor, CaughtUp, Declined, HistoryWatcher, OnSave, Shown, SuggestionId,
    UndoneChange, VersionsStatus, Worker,
};

use super::{Window, WindowInput, workbench};

/// How many versions the panel lists.
const LISTED: usize = 50;

/// One suggestion as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionRow {
    /// Its id.
    pub id: SuggestionId,
    /// What it does, in the AI app's words.
    pub title: String,
    /// The AI app that made it.
    pub by: String,
    /// Its patches, in order, as one patch file's text.
    pub patches: String,
    /// It holds clashing changes, so it can't be applied yet.
    pub clashing: bool,
}

/// One version as the panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRow {
    /// What it did.
    pub message: String,
    /// Who made it.
    pub by: String,
    /// When.
    pub time: String,
}

/// What the panel shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PanelShown {
    /// No project read yet.
    #[default]
    Nothing,
    /// Versions are off: they can be turned on.
    Off,
    /// The project sits inside another project's history.
    Nested {
        /// That project's folder.
        around: PathBuf,
    },
    /// Versions are on.
    On {
        /// The suggestions waiting.
        suggestions: Vec<SuggestionRow>,
        /// The versions, newest first.
        versions: Vec<VersionRow>,
    },
}

impl PanelShown {
    /// What the panel shows for what was read.
    pub fn of(shown: Shown) -> PanelShown {
        match shown.status {
            VersionsStatus::Off => PanelShown::Off,
            VersionsStatus::Nested { repository_root } => PanelShown::Nested {
                around: repository_root,
            },
            VersionsStatus::On => PanelShown::On {
                suggestions: shown
                    .suggestions
                    .into_iter()
                    .map(|suggestion| SuggestionRow {
                        clashing: !suggestion.conflicted.is_empty(),
                        patches: history::shown_patch(&suggestion.patches),
                        id: suggestion.id,
                        title: suggestion.title,
                        by: suggestion.agent.name,
                    })
                    .collect(),
                versions: shown
                    .versions
                    .into_iter()
                    .map(|version| VersionRow {
                        message: version.message,
                        by: version.actor.name,
                        time: version.time,
                    })
                    .collect(),
            },
        }
    }

    /// The page of the panel's stack that shows.
    fn page(&self) -> &'static str {
        match self {
            PanelShown::Nothing | PanelShown::Off => "off",
            PanelShown::Nested { .. } => "nested",
            PanelShown::On { .. } => "on",
        }
    }
}

/// The Versions panel's state: what it shows.
#[derive(Debug, Default)]
pub struct VersionsPanelComponent {
    shown: PanelShown,
}

/// What the Versions panel is told.
#[derive(Debug)]
pub enum VersionsPanelInput {
    /// Show this (the window read the project's versions).
    Show(PanelShown),
    /// People pressed Turn On.
    TurnOn,
    /// People pressed Undo.
    Undo,
    /// People pressed a suggestion's Show.
    ShowSuggestion(usize),
    /// People pressed a suggestion's Apply.
    Apply(usize),
    /// People pressed a suggestion's Discard.
    Discard(usize),
}

/// What the Versions panel tells the window: only what people did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionsPanelOutput {
    /// Turn versions on for the project.
    TurnOn,
    /// Undo the last change by anyone.
    Undo,
    /// Show a suggestion's patches as a ghost.
    Show(String),
    /// Apply a suggestion.
    Apply(SuggestionId),
    /// Discard a suggestion.
    Discard(SuggestionId),
}

/// A row's text: a title above a dimmed line.
fn row_text(title: &str, subtitle: &str) -> gtk::Box {
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    let subtitle = gtk::Label::builder()
        .label(subtitle)
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    text.append(&title);
    text.append(&subtitle);
    text
}

/// A small flat button sending `message` when pressed.
fn row_button(
    label: &str,
    tooltip: &str,
    input: &relm4::Sender<VersionsPanelInput>,
    message: impl Fn() -> VersionsPanelInput + 'static,
) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let input = input.clone();
    button.connect_clicked(move |_| input.emit(message()));
    button
}

/// A suggestion's row: its title and maker, then Show, Apply and Discard.
fn suggestion_row(
    index: usize,
    suggestion: &SuggestionRow,
    input: &relm4::Sender<VersionsPanelInput>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    let by = if suggestion.clashing {
        fill_in(
            &tr("From {app}. Its changes clash with the project as it is now."),
            &[("app", &suggestion.by)],
        )
    } else {
        fill_in(&tr("From {app}"), &[("app", &suggestion.by)])
    };
    row.append(&row_text(&suggestion.title, &by));
    row.append(&row_button(
        &tr("Show"),
        &tr("Show the suggestion on the model, without changing anything"),
        input,
        move || VersionsPanelInput::ShowSuggestion(index),
    ));
    let apply = row_button(
        &tr("Apply"),
        &tr("Make the suggestion part of the project"),
        input,
        move || VersionsPanelInput::Apply(index),
    );
    apply.set_sensitive(!suggestion.clashing);
    row.append(&apply);
    row.append(&row_button(
        &tr("Discard"),
        &tr("Throw the suggestion away"),
        input,
        move || VersionsPanelInput::Discard(index),
    ));
    row
}

/// A version's row: what it did, who and when.
fn version_row(version: &VersionRow) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_top(4);
    row.set_margin_bottom(4);
    let by = fill_in(
        &tr("{who}, {when}"),
        &[("who", &version.by), ("when", &version.time)],
    );
    row.append(&row_text(&version.message, &by));
    row
}

#[relm4::component(pub)]
impl Component for VersionsPanelComponent {
    type Init = ();
    type Input = VersionsPanelInput;
    type Output = VersionsPanelOutput;
    type CommandOutput = ();

    view! {
        gtk::Stack {
            add_named[Some("off")] = &adw::StatusPage {
                set_icon_name: Some("document-open-recent-symbolic"),
                set_title: &tr("Versions are off"),
                set_description: Some(&tr(
                    "Turn them on to keep every saved version of the project, undo any change, and see suggestions from AI apps here.",
                )),
                add_css_class: "compact",
                #[wrap(Some)]
                set_child = &gtk::Button {
                    set_label: &tr("Turn On"),
                    set_halign: gtk::Align::Center,
                    add_css_class: "pill",
                    add_css_class: "suggested-action",
                    connect_clicked[sender] => move |_| {
                        sender.input_sender().emit(VersionsPanelInput::TurnOn);
                    },
                },
            },
            add_named[Some("nested")] = &adw::StatusPage {
                set_icon_name: Some("document-open-recent-symbolic"),
                set_title: &tr("Versions can't be turned on here"),
                #[watch]
                set_description: Some(&model.nested_words()),
                add_css_class: "compact",
            },
            add_named[Some("on")] = &gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_vexpand: true,
                #[wrap(Some)]
                set_child = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 6,
                    set_margin_start: 12,
                    set_margin_end: 12,
                    set_margin_top: 8,
                    set_margin_bottom: 8,

                    gtk::Box {
                        set_orientation: gtk::Orientation::Horizontal,
                        gtk::Label {
                            set_label: &tr("Suggestions"),
                            set_xalign: 0.0,
                            set_hexpand: true,
                            add_css_class: "heading",
                        },
                        #[name = "undo"]
                        gtk::Button {
                            set_label: &tr("Undo"),
                            set_tooltip_text: Some(&tr("Undo the last change to the project, by anyone")),
                            add_css_class: "flat",
                            connect_clicked[sender] => move |_| {
                                sender.input_sender().emit(VersionsPanelInput::Undo);
                            },
                        },
                    },
                    #[name = "suggestions"]
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                    },
                    #[name = "no_suggestions"]
                    gtk::Label {
                        set_label: &tr("No suggestions. Ask an AI app for a change and it shows here."),
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "dim-label",
                    },
                    gtk::Label {
                        set_label: &tr("Versions"),
                        set_xalign: 0.0,
                        set_margin_top: 12,
                        add_css_class: "heading",
                    },
                    #[name = "versions"]
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                    },
                },
            },
            #[watch]
            set_visible_child_name: model.shown.page(),
        }
    }

    fn init((): (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = VersionsPanelComponent::default();
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: VersionsPanelInput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            VersionsPanelInput::Show(shown) => {
                self.shown = shown;
                self.fill(widgets, sender.input_sender());
            }
            VersionsPanelInput::TurnOn => {
                let _ = sender.output(VersionsPanelOutput::TurnOn);
            }
            VersionsPanelInput::Undo => {
                let _ = sender.output(VersionsPanelOutput::Undo);
            }
            VersionsPanelInput::ShowSuggestion(index) => {
                if let Some(suggestion) = self.suggestion(index) {
                    let _ = sender.output(VersionsPanelOutput::Show(suggestion.patches.clone()));
                }
            }
            VersionsPanelInput::Apply(index) => {
                if let Some(suggestion) = self.suggestion(index) {
                    let _ = sender.output(VersionsPanelOutput::Apply(suggestion.id.clone()));
                }
            }
            VersionsPanelInput::Discard(index) => {
                if let Some(suggestion) = self.suggestion(index) {
                    let _ = sender.output(VersionsPanelOutput::Discard(suggestion.id.clone()));
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

impl VersionsPanelComponent {
    /// The suggestion a row at `index` shows.
    fn suggestion(&self, index: usize) -> Option<&SuggestionRow> {
        match &self.shown {
            PanelShown::On { suggestions, .. } => suggestions.get(index),
            PanelShown::Nothing | PanelShown::Off | PanelShown::Nested { .. } => None,
        }
    }

    /// Why versions can't be turned on, naming the project around it.
    fn nested_words(&self) -> String {
        match &self.shown {
            PanelShown::Nested { around } => fill_in(
                &tr(
                    "This project is inside another project's history, at {folder}. A copy of the project in a folder of its own can have versions.",
                ),
                &[("folder", &around.display())],
            ),
            PanelShown::Nothing | PanelShown::Off | PanelShown::On { .. } => String::new(),
        }
    }

    /// Makes the rows again for what the panel shows (plain widgets, made
    /// once per reading: docs/relm4-migration.md, rule 12).
    fn fill(
        &self,
        widgets: &VersionsPanelComponentWidgets,
        input: &relm4::Sender<VersionsPanelInput>,
    ) {
        for list in [&widgets.suggestions, &widgets.versions] {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
        }
        let (suggestions, versions): (&[SuggestionRow], &[VersionRow]) = match &self.shown {
            PanelShown::On {
                suggestions,
                versions,
            } => (suggestions, versions),
            PanelShown::Nothing | PanelShown::Off | PanelShown::Nested { .. } => (&[], &[]),
        };
        for (index, suggestion) in suggestions.iter().enumerate() {
            widgets
                .suggestions
                .append(&suggestion_row(index, suggestion, input));
        }
        widgets.no_suggestions.set_visible(suggestions.is_empty());
        for version in versions {
            widgets.versions.append(&version_row(version));
        }
        widgets.undo.set_sensitive(!versions.is_empty());
    }
}

/// The Versions panel as the window holds it: the one owner of the
/// component's controller (dropping it shuts the component down), its
/// outputs forwarded to the window at launch.
pub struct VersionsPanel {
    /// The panel's content.
    pub root: gtk::Stack,
    controller: Controller<VersionsPanelComponent>,
}

impl VersionsPanel {
    /// An empty panel, each thing people do made into the owner's message
    /// by `map` and sent to `to`.
    pub fn new<X: 'static>(
        to: &relm4::Sender<X>,
        map: impl Fn(VersionsPanelOutput) -> X + 'static,
    ) -> VersionsPanel {
        let controller = VersionsPanelComponent::builder()
            .launch(())
            .forward(to, map);
        VersionsPanel {
            root: controller.widget().clone(),
            controller,
        }
    }

    /// Shows what the window read. Queued: it shows when the main loop
    /// next runs.
    pub fn show(&self, shown: PanelShown) {
        let _ = self
            .controller
            .sender()
            .send(VersionsPanelInput::Show(shown));
    }

    /// What the panel shows, for tests.
    #[cfg(test)]
    fn shown(&self) -> PanelShown {
        self.controller.model().shown.clone()
    }
}

// ---------------------------------------------------------------------------
// The window's side
// ---------------------------------------------------------------------------

/// What the window keeps about the project's versions.
#[derive(Debug, Default)]
pub(in crate::ui) struct WindowVersions {
    /// Watches the versions' folder while they are on.
    watcher: Option<HistoryWatcher>,
    /// Runs the work on the versions one job at a time, so a save never
    /// overlaps a catch-up.
    worker: Worker,
    /// The files the Code panel holds unsaved, as it last said.
    unsaved: BTreeSet<PathBuf>,
    /// Versions as last read.
    status: Option<VersionsStatus>,
    /// The question about turning versions on was answered (or put away)
    /// since the project was opened: it isn't asked again until then.
    asked: bool,
    /// The question shows now.
    asking: bool,
    /// Which reading of the versions is the latest asked for: an older
    /// one arriving late is not shown.
    reading: u64,
}

/// What the window is told about versions.
#[derive(Debug)]
pub(in crate::ui) enum VersionsInput {
    /// People did something in the Versions panel.
    Panel(VersionsPanelOutput),
    /// The Code panel saved these files.
    Saved(Vec<PathBuf>),
    /// Someone changed the project's versions (the window itself, or an AI
    /// app): catch up and read them again.
    Changed,
    /// The window came to the front: versions not watched yet may have
    /// been turned on meanwhile (by an AI app opening the project).
    Focused,
    /// People answered "Turn on versions for this project?".
    Answered(TurnOnAnswer),
    /// Work done off the main thread for this folder came back.
    Done {
        /// The folder it was done for.
        folder: PathBuf,
        /// What it did.
        done: Done,
    },
}

/// What people answered when asked to turn versions on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum TurnOnAnswer {
    /// Turn On.
    TurnOn,
    /// Not Now: remembered for this project.
    NotNow,
    /// The question was put away (Escape): asked again next time the
    /// project is opened.
    Later,
}

impl TurnOnAnswer {
    /// The answer an alert dialog's response id stands for.
    fn from_response(response: &str) -> TurnOnAnswer {
        match response {
            "turn-on" => TurnOnAnswer::TurnOn,
            "not-now" => TurnOnAnswer::NotNow,
            _ => TurnOnAnswer::Later,
        }
    }
}

/// What work done off the main thread did.
#[derive(Debug)]
pub(in crate::ui) enum Done {
    /// The versions were read (and the folder caught up first, when on).
    Read {
        /// Which reading it is.
        reading: u64,
        /// Catching up, when it ran.
        caught_up: Result<Option<CaughtUp>, String>,
        /// What was read.
        shown: Result<Shown, String>,
    },
    /// A save was merged into the project.
    Saved(Result<CaughtUp, String>),
    /// Versions were turned on.
    TurnedOn(Result<(), String>),
    /// A suggestion was applied: its title.
    Applied(Result<(String, CaughtUp), String>),
    /// A suggestion was discarded.
    Discarded(Result<(), String>),
    /// The last change was undone: what it was.
    Undone(Result<(UndoneChange, CaughtUp), String>),
}

/// The person using the window, as their versions name them: their full
/// name, or their login when the system knows none.
fn person() -> Actor {
    let real = glib::real_name().to_string_lossy().into_owned();
    let name = if real.trim().is_empty() || real == "Unknown" {
        glib::user_name().to_string_lossy().into_owned()
    } else {
        real
    };
    history::user_actor(&name)
}

/// Where "Not Now" is remembered.
fn declined() -> Declined {
    Declined::new(
        glib::user_config_dir()
            .join("stave")
            .join("versions-not-now"),
    )
}

impl WindowVersions {
    /// Keeps the files the Code panel holds unsaved, as it said.
    pub(in crate::ui) fn set_unsaved(&mut self, unsaved: BTreeSet<PathBuf>) {
        self.unsaved = unsaved;
    }
}

impl From<VersionsInput> for WindowInput {
    fn from(message: VersionsInput) -> WindowInput {
        WindowInput::Versions(message)
    }
}

/// Tells the window when it comes to the front ([`VersionsInput::Focused`]),
/// so versions turned on by an AI app while it was open are noticed
/// without a save.
pub(in crate::ui) fn wire(window: &Window) {
    let input = window.input();
    window
        .widgets
        .window
        .connect_is_active_notify(move |shown| {
            if shown.is_active() {
                input.emit(VersionsInput::Focused.into());
            }
        });
}

/// The Versions panel's outputs as the window's messages (forwarded at
/// launch).
pub(in crate::ui) fn panel_said(output: VersionsPanelOutput) -> WindowInput {
    VersionsInput::Panel(output).into()
}

impl Window {
    /// Handles a [`VersionsInput`].
    pub(in crate::ui) fn update_versions(&mut self, message: VersionsInput) {
        match message {
            VersionsInput::Panel(VersionsPanelOutput::TurnOn) => self.turn_versions_on(),
            VersionsInput::Panel(VersionsPanelOutput::Undo) => {
                let unsaved = self.unsaved_inside();
                self.in_background(
                    move |folder| history::undo(folder, &person(), &unsaved),
                    |result| Done::Undone(result.map(|undone| (undone.what, undone.caught_up))),
                );
            }
            VersionsInput::Panel(VersionsPanelOutput::Show(patches)) => {
                self.widgets.workbench.present(workbench::PanelId::View);
                self.dispatch(Command::Edit(EditCommand::ShowPatch(patches)));
            }
            VersionsInput::Panel(VersionsPanelOutput::Apply(id)) => {
                let unsaved = self.unsaved_inside();
                self.in_background(
                    move |folder| {
                        let applied = history::apply(folder, &id, &person())?;
                        let caught_up = history::catch_up(folder, &unsaved)?;
                        Ok((applied.title, caught_up))
                    },
                    Done::Applied,
                );
            }
            VersionsInput::Panel(VersionsPanelOutput::Discard(id)) => {
                self.in_background(
                    move |folder| history::discard(folder, &id, &person()),
                    Done::Discarded,
                );
            }
            VersionsInput::Saved(files) => self.versions_saved(files),
            VersionsInput::Changed => self.read_versions(),
            VersionsInput::Focused => {
                if self.versions.watcher.is_none() {
                    self.read_versions();
                }
            }
            VersionsInput::Answered(answer) => {
                self.versions.asking = false;
                self.versions.asked = true;
                match answer {
                    TurnOnAnswer::TurnOn => self.turn_versions_on(),
                    TurnOnAnswer::NotNow => {
                        if let Err(error) = declined().add(self.session.folder()) {
                            eprintln!("warning: {error}");
                        }
                    }
                    TurnOnAnswer::Later => {}
                }
            }
            VersionsInput::Done { folder, done } => {
                // Work for a folder no longer open is of no interest.
                if folder == self.session.folder() {
                    self.versions_done(done);
                }
            }
        }
    }

    /// A project was opened: what is known about the last one's versions
    /// goes, and this one's are watched and read (catching the folder up
    /// when someone changed them while it was closed).
    pub(in crate::ui) fn versions_opened(&mut self) {
        self.versions.watcher = None;
        self.versions.status = None;
        self.versions.asked = false;
        self.versions.unsaved.clear();
        self.widgets.versions.show(PanelShown::Nothing);
        self.watch_versions();
        self.read_versions();
    }

    /// Watches the project's versions when they are on; any change to them
    /// is [`VersionsInput::Changed`].
    fn watch_versions(&mut self) {
        let (sender, receiver) = async_channel::unbounded::<()>();
        self.versions.watcher = HistoryWatcher::start(self.session.folder(), move || {
            // The window may be gone; then nobody needs to know.
            let _ = sender.send_blocking(());
        });
        if self.versions.watcher.is_none() {
            return;
        }
        // Ends when the watcher goes (with the window, or for another
        // folder's), or once the window can't be told.
        let input = self.input();
        glib::spawn_future_local(async move {
            while receiver.recv().await.is_ok() {
                if input.send(VersionsInput::Changed.into()).is_err() {
                    break;
                }
            }
        });
    }

    /// The files the Code panel holds unsaved, inside the project.
    fn unsaved_inside(&self) -> BTreeSet<history::RelativePath> {
        history::unsaved_inside(self.session.folder(), &self.versions.unsaved)
    }

    /// Catches the folder up (when versions are on) and reads the versions
    /// again for the panel.
    fn read_versions(&mut self) {
        self.versions.reading = self.versions.reading.wrapping_add(1);
        let reading = self.versions.reading;
        let unsaved = self.unsaved_inside();
        self.in_background(
            move |folder| {
                let caught_up = match history::History::status(folder) {
                    Ok(VersionsStatus::On) => history::catch_up(folder, &unsaved)
                        .map(Some)
                        .map_err(|error| error.to_string()),
                    Ok(VersionsStatus::Off | VersionsStatus::Nested { .. }) => Ok(None),
                    Err(error) => Err(error.to_string()),
                };
                let shown = history::read(folder, LISTED).map_err(|error| error.to_string());
                Ok::<_, history::VersionsError>((caught_up, shown))
            },
            move |result| match result {
                Ok((caught_up, shown)) => Done::Read {
                    reading,
                    caught_up,
                    shown,
                },
                Err(error) => Done::Read {
                    reading,
                    caught_up: Err(error.clone()),
                    shown: Err(error),
                },
            },
        );
    }

    /// The Code panel saved files: they are merged into the project's
    /// versions, or, the first time for a project whose versions are off,
    /// people are asked whether to turn them on.
    fn versions_saved(&mut self, files: Vec<PathBuf>) {
        let folder = self.session.folder().to_path_buf();
        let status = match history::History::status(&folder) {
            Ok(status) => status,
            // A folder that can't be found has no versions to keep.
            Err(_) => return,
        };
        let declined = self.versions.asked || declined().contains(&folder);
        self.versions.status = Some(status.clone());
        match history::on_save(&status, declined) {
            OnSave::Record => {
                let unsaved = self.unsaved_inside();
                self.in_background(
                    move |folder| {
                        history::save(folder, &person(), &files, &unsaved)
                            .map(|saved| saved.caught_up)
                    },
                    Done::Saved,
                );
            }
            OnSave::AskToTurnOn => self.ask_to_turn_versions_on(),
            OnSave::Nothing => {}
        }
    }

    /// "Turn on versions for this project?", once.
    fn ask_to_turn_versions_on(&mut self) {
        if self.versions.asking {
            return;
        }
        self.versions.asking = true;
        let dialog = adw::AlertDialog::builder()
            .heading(tr("Turn on versions for this project?"))
            .body(tr(
                "Stave then keeps every saved version of the project in its folder, so any change can be undone, and shows suggestions from AI apps in the Versions panel.",
            ))
            .close_response("later")
            .build();
        dialog.add_responses(&[
            ("not-now", tr("_Not Now").as_str()),
            ("turn-on", tr("_Turn On").as_str()),
        ]);
        dialog.set_response_appearance("turn-on", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("turn-on"));
        let input = self.input();
        dialog.connect_response(None, move |_, response| {
            input.emit(VersionsInput::Answered(TurnOnAnswer::from_response(response)).into());
        });
        dialog.present(Some(&self.widgets.window));
    }

    /// Turns versions on, recording the project as it is.
    fn turn_versions_on(&mut self) {
        self.in_background(
            move |folder| history::turn_on(folder, &person()).map(|_| ()),
            Done::TurnedOn,
        );
    }

    /// Runs `work` on the project folder off the main thread; what it
    /// gives comes back as [`VersionsInput::Done`], made by `done`.
    fn in_background<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Path) -> Result<T, history::VersionsError> + Send + 'static,
        done: impl FnOnce(Result<T, String>) -> Done + 'static,
    ) {
        let folder = self.session.folder().to_path_buf();
        let input = self.input();
        let (sender, receiver) = async_channel::bounded(1);
        let working = folder.clone();
        let queued = self.versions.worker.run(move || {
            let result = work(&working).map_err(|error| error.to_string());
            // The window may be gone; then nobody needs to know.
            let _ = sender.send_blocking(result);
        });
        if let Err(stopped) = queued {
            eprintln!("warning: {stopped}");
        }
        glib::spawn_future_local(async move {
            let result = receiver
                .recv()
                .await
                .unwrap_or_else(|_| Err(tr("the work on the project's versions stopped")));
            let _ = input.send(
                VersionsInput::Done {
                    folder,
                    done: done(result),
                }
                .into(),
            );
        });
    }

    /// Work done off the main thread came back: says what happened, takes
    /// in what catching up wrote, and reads the versions again.
    fn versions_done(&mut self, done: Done) {
        match done {
            Done::Read {
                reading,
                caught_up,
                shown,
            } => {
                match caught_up {
                    Ok(Some(caught_up)) => self.caught_up(&caught_up),
                    Ok(None) => {}
                    Err(error) => eprintln!("warning: {error}"),
                }
                if reading != self.versions.reading {
                    return;
                }
                match shown {
                    Ok(shown) => {
                        // Versions turned on by someone else (an AI app
                        // opening the project) are watched from now on.
                        if shown.status == VersionsStatus::On && self.versions.watcher.is_none() {
                            self.watch_versions();
                        }
                        self.versions.status = Some(shown.status.clone());
                        self.widgets.versions.show(PanelShown::of(shown));
                    }
                    Err(error) => eprintln!("warning: {error}"),
                }
                return;
            }
            Done::Saved(result) => match result {
                Ok(caught_up) => self.caught_up(&caught_up),
                Err(error) => self.toast(&fill_in(
                    &tr("The save wasn't recorded in the project's versions: {reason}"),
                    &[("reason", &error)],
                )),
            },
            Done::TurnedOn(result) => match result {
                Ok(()) => {
                    self.toast(&tr("Versions are on for this project"));
                    self.watch_versions();
                }
                Err(error) => self.toast(&fill_in(
                    &tr("Versions couldn't be turned on: {reason}"),
                    &[("reason", &error)],
                )),
            },
            Done::Applied(result) => match result {
                Ok((title, caught_up)) => {
                    self.toast(&fill_in(&tr("Applied “{title}”"), &[("title", &title)]));
                    self.caught_up(&caught_up);
                }
                Err(error) => self.toast(&error),
            },
            Done::Discarded(result) => match result {
                Ok(()) => self.toast(&tr("Discarded the suggestion")),
                Err(error) => self.toast(&error),
            },
            Done::Undone(result) => match result {
                Ok((what, caught_up)) => {
                    self.toast(&what.said());
                    self.caught_up(&caught_up);
                }
                Err(error) => self.toast(&error),
            },
        }
        // Read again; the watcher would too, a moment later, but a window
        // whose versions were just turned on had no watcher yet.
        self.read_versions();
    }

    /// Takes in what catching up wrote: the design is checked again, which
    /// reloads the Code panel's clean files changed on disk; a clash, or
    /// unsaved files standing in the way, is said plainly.
    fn caught_up(&mut self, caught_up: &CaughtUp) {
        match caught_up {
            CaughtUp::UpToDate => {}
            CaughtUp::Wrote { conflicted, .. } => {
                if !conflicted.is_empty() {
                    let files = conflicted
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.toast(&fill_in(
                        &tr("Your changes and another change to the same lines clash in {files}: both are kept, between marker lines"),
                        &[("files", &files)],
                    ));
                }
                self.dispatch(Command::Check);
            }
            CaughtUp::Waiting { files } => {
                let files = files
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                self.toast(&fill_in(
                    &tr("{files} changed in the project; your unsaved changes are kept, and the two meet when you save"),
                    &[("files", &files)],
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, recorder, settle};
    use gtk::gio;

    fn on() -> PanelShown {
        PanelShown::On {
            suggestions: vec![
                SuggestionRow {
                    id: SuggestionId::new("aaaa"),
                    title: "Widen the drawer".to_owned(),
                    by: "Claude Desktop".to_owned(),
                    patches: "set drawer.width = 500mm\n".to_owned(),
                    clashing: false,
                },
                SuggestionRow {
                    id: SuggestionId::new("bbbb"),
                    title: "Add a shoe shelf".to_owned(),
                    by: "Codex".to_owned(),
                    patches: String::new(),
                    clashing: true,
                },
            ],
            versions: vec![VersionRow {
                message: "Saved dresser.stave".to_owned(),
                by: "Henry".to_owned(),
                time: "2026-11-20 10:00".to_owned(),
            }],
        }
    }

    /// Every button in `root`, by label, in order.
    fn buttons(root: &gtk::Widget) -> Vec<gtk::Button> {
        let mut found = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(widget) = pending.pop() {
            if let Ok(button) = widget.clone().downcast::<gtk::Button>() {
                found.push(button);
                continue;
            }
            let mut child = widget.last_child();
            while let Some(current) = child {
                child = current.prev_sibling();
                pending.push(current);
            }
        }
        found
    }

    fn labelled(root: &gtk::Widget, label: &str) -> Vec<gtk::Button> {
        buttons(root)
            .into_iter()
            .filter(|button| button.label().is_some_and(|text| text == label))
            .collect()
    }

    #[test]
    fn what_is_read_is_what_the_panel_shows() {
        let shown = Shown {
            status: VersionsStatus::Nested {
                repository_root: PathBuf::from("/home/me/work"),
            },
            suggestions: Vec::new(),
            versions: Vec::new(),
        };
        assert_eq!(
            PanelShown::of(shown),
            PanelShown::Nested {
                around: PathBuf::from("/home/me/work")
            }
        );
        assert_eq!(PanelShown::Off.page(), "off");
        assert_eq!(on().page(), "on");
    }

    #[test]
    fn the_panel_lists_suggestions_and_says_what_people_press() {
        on_gtk_thread(lists_and_presses);
    }

    /// Shown, the panel lists each suggestion with Show, Apply and
    /// Discard (Apply not offered while it clashes) and each version; its
    /// buttons tell the owner what people pressed, by suggestion id.
    fn lists_and_presses() {
        let (to, heard) = recorder::<VersionsPanelOutput>();
        let panel = VersionsPanel::new(&to, |output| output);
        panel.show(on());
        settle();
        assert_eq!(panel.shown(), on());
        let root = panel.root.clone().upcast::<gtk::Widget>();
        assert_eq!(panel.root.visible_child_name().as_deref(), Some("on"));
        let applies = labelled(&root, "Apply");
        assert_eq!(applies.len(), 2);
        assert!(
            applies
                .first()
                .is_some_and(gtk::prelude::WidgetExt::is_sensitive)
        );
        assert!(
            !applies
                .get(1)
                .is_some_and(gtk::prelude::WidgetExt::is_sensitive),
            "a clashing suggestion can't be applied"
        );
        assert!(heard.borrow().is_empty(), "showing says nothing");

        if let Some(button) = labelled(&root, "Show").first() {
            button.emit_clicked();
        }
        if let Some(button) = applies.first() {
            button.emit_clicked();
        }
        if let Some(button) = labelled(&root, "Discard").get(1) {
            button.emit_clicked();
        }
        for button in labelled(&root, "Undo") {
            button.emit_clicked();
        }
        assert!(heard.borrow().is_empty(), "only queued");
        settle();
        assert_eq!(
            *heard.borrow(),
            vec![
                VersionsPanelOutput::Show("set drawer.width = 500mm\n".to_owned()),
                VersionsPanelOutput::Apply(SuggestionId::new("aaaa")),
                VersionsPanelOutput::Discard(SuggestionId::new("bbbb")),
                VersionsPanelOutput::Undo,
            ]
        );
    }

    #[test]
    fn an_off_project_offers_to_turn_versions_on() {
        on_gtk_thread(off_offers);
    }

    fn off_offers() {
        let (to, heard) = recorder::<VersionsPanelOutput>();
        let panel = VersionsPanel::new(&to, |output| output);
        panel.show(PanelShown::Off);
        settle();
        assert_eq!(panel.root.visible_child_name().as_deref(), Some("off"));
        let root = panel.root.clone().upcast::<gtk::Widget>();
        for button in labelled(&root, "Turn On") {
            button.emit_clicked();
        }
        settle();
        assert_eq!(*heard.borrow(), vec![VersionsPanelOutput::TurnOn]);
    }

    #[test]
    fn a_dropped_panel_is_freed() {
        on_gtk_thread(panel_is_freed);
    }

    /// Once the owner lets go of the panel, its component, widgets and the
    /// owner's callback are freed, with rows made.
    fn panel_is_freed() {
        let token = std::rc::Rc::new(());
        let held = std::rc::Rc::clone(&token);
        let to = crate::ui::gtk_test::listener(move |_: VersionsPanelOutput| {
            let _ = &held;
        });
        let panel = VersionsPanel::new(&to, |output| output);
        drop(to);
        let callback = std::rc::Rc::downgrade(&token);
        drop(token);
        panel.show(on());
        settle();
        let root = panel.root.downgrade();
        let input = panel.controller.sender().clone();
        drop(panel);
        settle();
        assert!(root.upgrade().is_none(), "its widget is freed");
        assert!(
            input.send(VersionsPanelInput::Undo).is_err(),
            "the component's runtime is gone"
        );
        assert!(callback.upgrade().is_none(), "the callback is freed");
    }

    #[test]
    fn answers_are_read_from_the_dialog_s_responses() {
        assert_eq!(TurnOnAnswer::from_response("turn-on"), TurnOnAnswer::TurnOn);
        assert_eq!(TurnOnAnswer::from_response("not-now"), TurnOnAnswer::NotNow);
        assert_eq!(TurnOnAnswer::from_response("later"), TurnOnAnswer::Later);
    }

    /// Runs the main loop, a little at a time, until `done` holds (work
    /// off the main thread comes back as messages); false if it never
    /// does.
    fn settle_until(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            settle();
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        false
    }

    /// A window on a copy of the wall shelf in a folder of its own (not
    /// inside any repository), in an application of its own.
    fn open_copy(id: &str) -> (adw::Application, relm4::Controller<Window>, PathBuf) {
        use crate::ui::{Options, WindowInit, build_widgets};
        let from = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/wall_shelf"
        ));
        let to = std::env::temp_dir().join(format!("stave-versions-test-{id}"));
        let _ = std::fs::remove_dir_all(&to);
        std::fs::create_dir_all(&to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_file() {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
        let folder = to.canonicalize().unwrap();
        let application = adw::Application::new(Some(id), gio::ApplicationFlags::NON_UNIQUE);
        application.register(gio::Cancellable::NONE).unwrap();
        let options = Options {
            layout: Some(std::env::temp_dir().join("stave-versions-test-no-layout")),
            ..Options::default()
        };
        let builder = Window::builder();
        let root = builder.widget().clone();
        let widgets =
            build_widgets(&root, &application, &folder, &gio::Menu::new(), &options).unwrap();
        let controller = builder
            .launch(WindowInit {
                widgets,
                folder: folder.clone(),
                options,
            })
            .detach();
        settle();
        (application, controller, folder)
    }

    /// Types a line at the end of the shelf's file in the Code panel and
    /// saves it with the window's Save.
    fn type_and_save(controller: &relm4::Controller<Window>, folder: &Path, line: &str) {
        let path = folder.join("shelf.stave");
        controller
            .model()
            .widgets
            .files
            .open_document(&stave_ide::Document::File(path.clone()));
        settle();
        let buffer = controller.model().widgets.files.open_buffer(&path).unwrap();
        buffer.insert(&mut buffer.end_iter(), line);
        settle();
        let _ = controller
            .sender()
            .send(crate::ui::update::project::ProjectInput::Save.into());
        settle();
    }

    #[test]
    fn the_first_save_asks_to_turn_versions_on_and_later_saves_are_recorded() {
        on_gtk_thread(first_save_asks);
    }

    /// A project without versions: its first save asks "Turn on versions
    /// for this project?"; Turn On records the project, the panel lists it,
    /// and the next save is recorded as the person's. An AI app applying a
    /// change then reaches the folder by itself.
    fn first_save_asks() {
        let (application, controller, folder) = open_copy("org.stave.VersionsTest.FirstSaveAsks");
        let root = controller.widget().clone();
        assert!(
            settle_until(|| controller.model().widgets.versions.shown() == PanelShown::Off),
            "an off project offers to turn versions on"
        );
        type_and_save(&controller, &folder, "\n// first\n");
        let Some(dialog) = root.visible_dialog() else {
            panic!("the first save asks about versions");
        };
        let alert = dialog.downcast::<adw::AlertDialog>().unwrap();
        assert_eq!(
            alert.heading().as_deref(),
            Some("Turn on versions for this project?")
        );
        alert.emit_by_name::<()>("response", &[&"turn-on"]);
        alert.force_close();
        let listed = |controller: &relm4::Controller<Window>| match controller
            .model()
            .widgets
            .versions
            .shown()
        {
            PanelShown::On { versions, .. } => versions.len(),
            PanelShown::Nothing | PanelShown::Off | PanelShown::Nested { .. } => 0,
        };
        assert!(
            settle_until(|| listed(&controller) == 1),
            "turned on, with the project recorded"
        );
        assert!(folder.join(".stave/history/repo").is_dir());

        type_and_save(&controller, &folder, "// second\n");
        assert!(root.visible_dialog().is_none(), "asked only once");
        assert!(
            settle_until(|| listed(&controller) == 2),
            "the save is recorded"
        );
        let history = history::History::open(&folder).unwrap();
        let latest = history.versions(1).unwrap();
        assert_eq!(
            latest.first().map(|version| version.message.as_str()),
            Some("Saved shelf.stave")
        );

        // An AI app applies a change to the notes: the window catches up.
        let main = history.main().unwrap();
        let mut files = history.files_at(&main).unwrap();
        let notes = history::RelativePath::new(Path::new("notes.txt")).unwrap();
        files.insert(notes, b"from an AI app\n".to_vec());
        let agent = Actor {
            kind: history::ActorKind::Agent,
            name: "Claude Desktop".to_owned(),
            id: "3f2a".to_owned(),
        };
        let mut history = history;
        let id = history
            .suggest(&agent, None, &files, "a patch", "Add notes")
            .unwrap();
        history.apply(&id, &agent, &main, &files).unwrap();
        assert!(
            settle_until(|| std::fs::read_to_string(folder.join("notes.txt"))
                .is_ok_and(|text| text == "from an AI app\n")),
            "the window wrote the applied change into the folder"
        );

        root.destroy();
        drop(controller);
        settle();
        drop(application);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn versions_an_ai_app_turns_on_are_watched_once_noticed() {
        on_gtk_thread(turned_on_elsewhere);
    }

    /// An AI app turns versions on while the window is open: once the
    /// window comes to the front it lists them and watches them, so a
    /// change the app applies reaches the folder without a save.
    fn turned_on_elsewhere() {
        let (application, controller, folder) =
            open_copy("org.stave.VersionsTest.TurnedOnElsewhere");
        let root = controller.widget().clone();
        assert!(
            settle_until(|| controller.model().widgets.versions.shown() == PanelShown::Off),
            "an off project offers to turn versions on"
        );
        let agent = Actor {
            kind: history::ActorKind::Agent,
            name: "Claude Desktop".to_owned(),
            id: "3f2a".to_owned(),
        };
        history::turn_on(&folder, &agent).unwrap();
        let _ = controller.sender().send(VersionsInput::Focused.into());
        assert!(
            settle_until(|| matches!(
                controller.model().widgets.versions.shown(),
                PanelShown::On { .. }
            )),
            "the panel shows the versions"
        );

        let mut history = history::History::open(&folder).unwrap();
        let main = history.main().unwrap();
        let mut files = history.files_at(&main).unwrap();
        let notes = history::RelativePath::new(Path::new("notes.txt")).unwrap();
        files.insert(notes, b"from an AI app\n".to_vec());
        let id = history
            .suggest(&agent, None, &files, "a patch", "Add notes")
            .unwrap();
        history.apply(&id, &agent, &main, &files).unwrap();
        assert!(
            settle_until(|| std::fs::read_to_string(folder.join("notes.txt"))
                .is_ok_and(|text| text == "from an AI app\n")),
            "the window watches the versions and catches up"
        );

        root.destroy();
        drop(controller);
        settle();
        drop(application);
        let _ = std::fs::remove_dir_all(folder);
    }
}
