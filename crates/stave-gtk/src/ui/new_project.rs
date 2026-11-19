//! A new project (`docs/editing.md`, "Naming rules", Envelope; task 7.1):
//! a dialog asks for the name, the three overall sizes, the wood and the
//! tools, the new-project writer (`stave_app::change::new_project_file`)
//! writes it into a folder the user chooses, and the window opens it with
//! the envelope lit as the first space to click.
//!
//! ```text
//! ┌ New Project ─────────────────────────────── [Cancel] [Create] ┐
//! │ Name                 [Hall table              ]               │
//! │ OVERALL SIZE  Width [900] mm  Height [760] mm  Depth [400] mm │
//! │ WOOD   Called [oak_19]  Species [white oak ▾]  Bought as …    │
//! │ LIBRARIES   [x] casework  [x] frames                          │
//! │ TOOLS YOU HAVE (none ticked: every tool)  [ ] tenon saw …     │
//! │ FOLDER   ~/Projects/hall_table           [Choose Folder…]     │
//! └───────────────────────────────────────────────────────────────┘
//! ```
//!
//! The lists come from the language, never from the window: the species
//! from `stave_schema`, the libraries a project imports from
//! `stave_modules`, the tools from the standard library's tool kinds.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt};
use stave_app::candidates::new_project::{BoardForm, NewProject, NewStock, StockCut};
use stave_app::change::new_project_file;
use stave_app::editing::{parse_name, write_all};
use stave_app::naming::ENVELOPE_REGION;
use stave_app::scratch::DiskWriter;
use stave_lang::ast::{BaseKind, ExprKind, Ident, ItemKind, StringPart};
use stave_lang::units::Length;
use stave_lang::{SourceMap, parse_file};
use stave_modules::{PRELUDE_MODULES, STD_MODULES, std_files};
use stave_schema::species::Species;

use super::Window;
use super::relm4_shim::Bridge;

/// A tool kind of the standard library: its name and its title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ToolChoice {
    /// The kind's name, as a project's `tools:` list writes it.
    pub(super) name: Ident,
    /// Its title, for the check box: "tenon saw".
    pub(super) title: String,
}

/// The standard library's tool kinds, in the order it declares them.
pub(super) fn standard_tools() -> Vec<ToolChoice> {
    let mut tools = Vec::new();
    let mut sources = SourceMap::new();
    for (module, file, text) in std_files() {
        if module != "tools" {
            continue;
        }
        let Ok(id) = sources.add(file, text) else {
            continue;
        };
        let Some(ast) = parse_file(id, text).ast else {
            continue;
        };
        for item in ast.items {
            let ItemKind::Kind(kind) = item.kind else {
                continue;
            };
            if kind.base.value != BaseKind::Tool || item.private.is_some() {
                continue;
            }
            let title = kind
                .body
                .field("title")
                .and_then(|field| match &field.value.kind {
                    ExprKind::String(literal) => Some(
                        literal
                            .parts
                            .iter()
                            .filter_map(|part| match part {
                                StringPart::Text { text, .. } => Some(text.as_str()),
                                StringPart::Interp { .. } => None,
                            })
                            .collect::<String>(),
                    ),
                    _ => None,
                })
                .unwrap_or_else(|| kind.name.as_str().replace('_', " "));
            tools.push(ToolChoice {
                name: kind.name,
                title,
            });
        }
    }
    tools
}

/// The standard libraries a project imports to use (not in the prelude):
/// `std:casework`, `std:frames`.
pub(super) fn importable_libraries() -> Vec<&'static str> {
    STD_MODULES
        .into_iter()
        .filter(|module| !PRELUDE_MODULES.contains(module))
        .collect()
}

/// What the dialog's fields hold, read as a new project.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Answers {
    /// The project's title.
    pub(super) title: String,
    /// Width, height and depth, millimetres.
    pub(super) size: [f64; 3],
    /// The stock's name.
    pub(super) stock_name: String,
    /// Its species.
    pub(super) species: Species,
    /// Solid, plywood or MDF.
    pub(super) form: BoardForm,
    /// Its thickness and the longest board, millimetres.
    pub(super) thickness: f64,
    /// The longest board or sheet you can get, millimetres.
    pub(super) max_length: f64,
    /// The libraries ticked, as `std:<name>`.
    pub(super) imports: Vec<String>,
    /// The tools ticked.
    pub(super) tools: Vec<Ident>,
}

/// A whole number of millimetres as a length.
fn millimetres(value: f64) -> Result<Length, String> {
    let rounded = value.round();
    if !(1.0..=100_000.0).contains(&rounded) {
        return Err(format!("{value} mm isn't a size Stave can use"));
    }
    // Checked above: a whole number from 1 to 100 000.
    let whole = format!("{rounded:.0}")
        .parse::<i64>()
        .map_err(|error| error.to_string())?;
    Ok(Length::from_mm(whole))
}

impl Answers {
    /// The new project these answers describe, or what is missing, in
    /// words.
    pub(super) fn project(&self) -> Result<NewProject, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Give the project a name".to_owned());
        }
        let name = parse_name(self.stock_name.trim()).map_err(|_| {
            format!(
                "'{}' can't name the wood: use letters, digits and _",
                self.stock_name.trim()
            )
        })?;
        let [width, height, depth] = self.size;
        Ok(NewProject {
            title: title.to_owned(),
            size: [
                millimetres(width)?,
                millimetres(height)?,
                millimetres(depth)?,
            ],
            stock: vec![NewStock {
                name,
                species: self.species,
                cut: StockCut::Board {
                    form: self.form,
                    thickness: millimetres(self.thickness)?,
                    width: None,
                    max_width: None,
                    max_length: millimetres(self.max_length)?,
                },
            }],
            tools: self.tools.clone(),
            imports: self.imports.clone(),
        })
    }
}

/// The forms of stock a new project's wood is bought in, with their words.
const FORMS: [(BoardForm, &str); 3] = [
    (BoardForm::Solid, "solid boards"),
    (BoardForm::Plywood, "plywood sheets"),
    (BoardForm::Mdf, "MDF sheets"),
];

/// The New Project dialog's fields: plain widgets that hold what people
/// type, read as [`Answers`] when they press Create.
#[derive(Clone, Debug)]
pub(super) struct Form {
    title: adw::EntryRow,
    sizes: [adw::SpinRow; 3],
    stock_name: adw::EntryRow,
    species: adw::ComboRow,
    form: adw::ComboRow,
    thickness: adw::SpinRow,
    max_length: adw::SpinRow,
    libraries: Vec<(&'static str, gtk::CheckButton)>,
    tools: Vec<(ToolChoice, gtk::CheckButton)>,
    /// The groups the fields sit in.
    general: adw::PreferencesGroup,
    size: adw::PreferencesGroup,
    wood: adw::PreferencesGroup,
    libraries_group: adw::PreferencesGroup,
    tools_group: adw::PreferencesGroup,
}

fn spin(title: &str, value: f64, low: f64, high: f64) -> adw::SpinRow {
    let row = adw::SpinRow::with_range(low, high, 1.0);
    row.set_title(title);
    row.set_value(value);
    row.set_digits(0);
    row
}

fn group(title: &str, description: Option<&str>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    if let Some(description) = description {
        group.set_description(Some(description));
    }
    group
}

impl Form {
    /// The fields, filled with a small table's sizes and oak.
    fn new() -> Form {
        let title = adw::EntryRow::builder().title("Name").build();
        title.set_text("Hall table");
        let sizes = [
            spin("Width (across), mm", 900.0, 10.0, 5000.0),
            spin("Height (up), mm", 760.0, 10.0, 5000.0),
            spin("Depth (front to back), mm", 400.0, 10.0, 5000.0),
        ];
        let stock_name = adw::EntryRow::builder().title("Called").build();
        stock_name.set_text("oak_19");
        let species_words: Vec<String> = Species::ALL
            .iter()
            .map(|species| species.word().replace('_', " "))
            .collect();
        let species_list =
            gtk::StringList::new(&species_words.iter().map(String::as_str).collect::<Vec<_>>());
        let species = adw::ComboRow::builder()
            .title("Species")
            .model(&species_list)
            .build();
        let form_list = gtk::StringList::new(&FORMS.map(|(_, words)| words));
        let form = adw::ComboRow::builder()
            .title("Bought as")
            .model(&form_list)
            .build();
        let thickness = spin("Thickness, mm", 19.0, 1.0, 200.0);
        let max_length = spin("Longest board you can get, mm", 2400.0, 100.0, 6000.0);

        let general = group("Project", None);
        general.add(&title);
        let size = group(
            "Overall size",
            Some("The box the piece fits in: the first space you click to add to"),
        );
        for row in &sizes {
            size.add(row);
        }
        let wood = group(
            "Wood",
            Some("The stock you buy; add more later in the text"),
        );
        wood.add(&stock_name);
        wood.add(&species);
        wood.add(&form);
        wood.add(&thickness);
        wood.add(&max_length);

        let libraries_group = group("Libraries", Some("What the menus offer to put in a space"));
        let libraries: Vec<(&'static str, gtk::CheckButton)> = importable_libraries()
            .into_iter()
            .map(|module| {
                let check = gtk::CheckButton::builder().active(true).build();
                let row = adw::ActionRow::builder()
                    .title(module)
                    .subtitle(format!("import \"std:{module}\""))
                    .activatable_widget(&check)
                    .build();
                row.add_prefix(&check);
                libraries_group.add(&row);
                (module, check)
            })
            .collect();

        let tools_group = group(
            "Tools you have",
            Some("Leave them all unticked if you have every tool"),
        );
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(4)
            .min_children_per_line(2)
            .column_spacing(6)
            .row_spacing(2)
            .build();
        let tools: Vec<(ToolChoice, gtk::CheckButton)> = standard_tools()
            .into_iter()
            .map(|tool| {
                let check = gtk::CheckButton::builder().label(&tool.title).build();
                check.set_tooltip_text(Some(tool.name.as_str()));
                flow.insert(&check, -1);
                (tool, check)
            })
            .collect();
        tools_group.add(&flow);

        Form {
            title,
            sizes,
            stock_name,
            species,
            form,
            thickness,
            max_length,
            libraries,
            tools,
            general,
            size,
            wood,
            libraries_group,
            tools_group,
        }
    }

    /// What the fields hold.
    fn answers(&self) -> Answers {
        let index = |row: &adw::ComboRow| usize::try_from(row.selected()).unwrap_or(0);
        Answers {
            title: self.title.text().to_string(),
            size: self.sizes.each_ref().map(adw::SpinRow::value),
            stock_name: self.stock_name.text().to_string(),
            species: Species::ALL
                .get(index(&self.species))
                .copied()
                .unwrap_or(Species::WhiteOak),
            form: FORMS
                .get(index(&self.form))
                .map_or(BoardForm::Solid, |(form, _)| *form),
            thickness: self.thickness.value(),
            max_length: self.max_length.value(),
            imports: self
                .libraries
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(module, _)| format!("std:{module}"))
                .collect(),
            tools: self
                .tools
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(tool, _)| tool.name.clone())
                .collect(),
        }
    }
}

/// The New Project dialog's state: the fields, the folder chosen and
/// what is wrong, if anything.
#[derive(Debug)]
pub(super) struct NewProjectComponent {
    form: Form,
    /// The folder the project is written into.
    folder: Option<PathBuf>,
    /// What is wrong, said under the fields.
    error: Option<String>,
}

/// What the New Project dialog is told.
#[derive(Debug)]
pub(super) enum NewProjectInput {
    /// People pressed Cancel.
    Cancel,
    /// People pressed Choose Folder….
    ChooseFolder,
    /// People chose the folder to write the project into.
    FolderChosen(PathBuf),
    /// People pressed Create.
    Create,
    /// The dialog has closed.
    Closed,
}

/// What the New Project dialog tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum NewProjectOutput {
    /// The project was written into this folder, and the dialog closed.
    Created(PathBuf),
    /// The dialog has closed, with a project written or not.
    Closed,
}

impl NewProjectComponent {
    /// Writes the project the fields describe into the folder chosen:
    /// the folder on success, or what is wrong in words.
    fn create(&self) -> Result<PathBuf, String> {
        let project = self.form.answers().project()?;
        let folder = self
            .folder
            .clone()
            .ok_or_else(|| "Choose a folder to write the project into".to_owned())?;
        create_project(&folder, &project)?;
        Ok(folder)
    }
}

/// The words beside Choose Folder….
fn folder_words(folder: Option<&std::path::Path>) -> String {
    folder.map_or_else(
        || "No folder chosen yet".to_owned(),
        |folder| folder.display().to_string(),
    )
}

#[relm4::component(pub(super))]
impl Component for NewProjectComponent {
    type Init = Form;
    type Input = NewProjectInput;
    type Output = NewProjectOutput;
    type CommandOutput = ();

    view! {
        adw::Dialog {
            set_title: "New Project",
            set_content_width: 560,
            set_content_height: 720,
            connect_closed[sender] => move |_| {
                sender.input_sender().emit(NewProjectInput::Closed);
            },

            #[wrap(Some)]
            set_child = &adw::ToolbarView {
                add_top_bar = &adw::HeaderBar {
                    set_show_start_title_buttons: false,
                    set_show_end_title_buttons: false,
                    pack_start = &gtk::Button {
                        set_label: "Cancel",
                        connect_clicked[sender] => move |_| {
                            sender.input_sender().emit(NewProjectInput::Cancel);
                        },
                    },
                    pack_end = &gtk::Button {
                        set_label: "Create",
                        add_css_class: "suggested-action",
                        connect_clicked[sender] => move |_| {
                            sender.input_sender().emit(NewProjectInput::Create);
                        },
                    },
                },

                #[wrap(Some)]
                set_content = &gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,

                    adw::PreferencesPage {
                        add: &model.form.general,
                        add: &model.form.size,
                        add: &model.form.wood,
                        add: &model.form.libraries_group,
                        add: &model.form.tools_group,
                        add = &adw::PreferencesGroup {
                            set_title: "Folder",
                            set_description: Some("The project's .stave file is written here"),
                            add = &adw::ActionRow {
                                set_title: "Write it into",
                                add_suffix = &gtk::Label {
                                    set_xalign: 0.0,
                                    set_hexpand: true,
                                    set_ellipsize: gtk::pango::EllipsizeMode::Start,
                                    #[watch]
                                    set_label: &folder_words(model.folder.as_deref()),
                                    #[watch]
                                    set_class_active: ("dim-label", model.folder.is_none()),
                                },
                                add_suffix = &gtk::Button {
                                    set_label: "Choose Folder…",
                                    connect_clicked[sender] => move |_| {
                                        sender.input_sender().emit(NewProjectInput::ChooseFolder);
                                    },
                                },
                            },
                        },
                    },
                    gtk::Label {
                        set_xalign: 0.0,
                        set_wrap: true,
                        add_css_class: "error",
                        set_margin_start: 24,
                        set_margin_end: 24,
                        set_margin_bottom: 12,
                        #[watch]
                        set_label: model.error.as_deref().unwrap_or_default(),
                        #[watch]
                        set_visible: model.error.is_some(),
                    },
                },
            },
        }
    }

    fn init(form: Form, root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let model = NewProjectComponent {
            form,
            folder: None,
            error: None,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: NewProjectInput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NewProjectInput::Cancel => {
                root.close();
            }
            NewProjectInput::ChooseFolder => {
                let parent = root
                    .root()
                    .and_then(|window| window.downcast::<gtk::Window>().ok());
                let chooser = gtk::FileDialog::builder()
                    .title("Choose the folder for the new project")
                    .modal(true)
                    .build();
                let input = sender.input_sender().clone();
                chooser.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
                    if let Ok(folder) = result
                        && let Some(path) = folder.path()
                    {
                        input.emit(NewProjectInput::FolderChosen(path));
                    }
                });
            }
            NewProjectInput::FolderChosen(folder) => {
                self.folder = Some(folder);
            }
            NewProjectInput::Create => match self.create() {
                Ok(folder) => {
                    self.error = None;
                    root.close();
                    // Nobody listening is not a mistake: the owner may
                    // not want to hear.
                    let _ = sender.output(NewProjectOutput::Created(folder));
                }
                Err(words) => self.error = Some(words),
            },
            NewProjectInput::Closed => {
                let _ = sender.output(NewProjectOutput::Closed);
            }
        }
    }
}

/// The New Project dialog, behind the API its callers still use: it owns
/// the component (dropping it shuts the component down).
pub(super) struct NewProjectDialog {
    /// The dialog.
    pub(super) dialog: adw::Dialog,
    bridge: Bridge<NewProjectComponent>,
}

impl NewProjectDialog {
    /// The dialog, filled with a small table's sizes and oak.
    pub(super) fn new() -> NewProjectDialog {
        let bridge = Bridge::<NewProjectComponent>::launch(Form::new());
        NewProjectDialog {
            dialog: bridge.widget().clone(),
            bridge,
        }
    }

    /// What the fields hold.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "kept for the dialog's API; only tests read the fields from outside now"
        )
    )]
    pub(super) fn answers(&self) -> Answers {
        self.bridge.model().form.answers()
    }
}

/// Writes a new project into `folder` with the editor's one writer, never
/// over a file that is there. Returns the file written.
pub(super) fn create_project(
    folder: &std::path::Path,
    project: &NewProject,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(folder)
        .map_err(|error| format!("can't make the folder '{}': {error}", folder.display()))?;
    let file = new_project_file(folder, project).map_err(|error| error.to_string())?;
    write_all(std::slice::from_ref(&file), &mut DiskWriter).map_err(|error| error.to_string())?;
    Ok(file.path)
}

impl Window {
    /// Asks for a new project and, once it is written, opens it in this
    /// window.
    pub(super) fn ask_new_project(self: &Rc<Self>) {
        let window = self.clone();
        ask(self.widgets.window.upcast_ref(), move |folder| {
            let next = window.clone();
            window.ask_about_unsaved(move || {
                next.open_folder(folder);
                next.welcome_new_project();
            });
        });
    }

    /// A project just made: the envelope lit as the first space to click.
    pub(super) fn welcome_new_project(&self) {
        self.light_first_space(ENVELOPE_REGION);
        self.show_hint("Click inside the box to add the first piece");
    }
}

/// Asks for a new project over `parent` and writes it; `created` gets the
/// folder it was written into.
pub(super) fn ask(parent: &gtk::Window, created: impl Fn(PathBuf) + 'static) {
    let form = NewProjectDialog::new();
    let dialog = form.dialog.clone();
    // Nothing else owns the dialog while it shows, so its own output
    // callback keeps it until it says it has closed, and then lets it go
    // (which shuts the component down).
    let open: Rc<Cell<Option<NewProjectDialog>>> = Rc::default();
    let keep = Rc::clone(&open);
    form.bridge.connect(move |output| match output {
        NewProjectOutput::Created(folder) => created(folder),
        NewProjectOutput::Closed => drop(keep.take()),
    });
    open.set(Some(form));
    dialog.present(Some(parent));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// The dialog's buttons, by their words.
    fn button(widget: &gtk::Widget, words: &str) -> Option<gtk::Button> {
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Ok(button) = current.clone().downcast::<gtk::Button>()
                && button.label().as_deref() == Some(words)
            {
                return Some(button);
            }
            if let Some(found) = button(&current, words) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }

    #[test]
    fn a_dropped_new_project_dialog_is_freed() {
        on_gtk_thread(dialog_is_freed);
    }

    /// Dropping the dialog shuts its component down when the main loop
    /// next runs: the dialog, its fields and the model's runtime are
    /// freed.
    fn dialog_is_freed() {
        let form = NewProjectDialog::new();
        let dialog = form.dialog.downgrade();
        let title = form.bridge.model().form.title.downgrade();
        let input = form.bridge.sender().clone();
        settle();
        assert!(dialog.upgrade().is_some(), "the dialog lives while held");
        drop(form);
        settle();
        assert!(dialog.upgrade().is_none(), "the dialog is freed");
        assert!(title.upgrade().is_none(), "its fields are freed");
        assert!(
            input.send(NewProjectInput::Create).is_err(),
            "the component's runtime, and with it the model, is gone"
        );
    }

    #[test]
    fn an_asked_dialog_lives_while_it_shows_and_is_freed_once_closed() {
        on_gtk_thread(asked_dialog_is_freed_once_closed);
    }

    /// Nothing else owns the dialog `ask` shows: it keeps itself while it
    /// shows, and lets itself go once it has closed.
    fn asked_dialog_is_freed_once_closed() {
        let window = adw::Window::new();
        ask(window.upcast_ref(), |_| {});
        let Some(dialog) = window.visible_dialog() else {
            panic!("the dialog shows over the window");
        };
        let shown = dialog.downgrade();
        drop(dialog);
        settle();
        assert!(shown.upgrade().is_some(), "the dialog lives while it shows");
        if let Some(dialog) = shown.upgrade() {
            dialog.force_close();
        }
        settle();
        assert!(shown.upgrade().is_none(), "the dialog is freed once closed");
        window.destroy();
    }

    #[test]
    fn create_without_a_folder_says_so_once_the_loop_runs() {
        on_gtk_thread(create_is_queued);
    }

    /// Pressing Create only queues a message: what is wrong shows when
    /// the main loop runs, and a choice of folder made meanwhile (as the
    /// file chooser's answer would be) is handled in order after it.
    fn create_is_queued() {
        let form = NewProjectDialog::new();
        let heard = Rc::new(Cell::new(0));
        let count = Rc::clone(&heard);
        form.bridge.connect(move |_| count.set(count.get() + 1));
        // A dialog not yet presented holds its content apart.
        let Some(create) = form
            .dialog
            .child()
            .and_then(|content| button(&content, "Create"))
        else {
            panic!("the dialog has a Create button");
        };
        create.emit_clicked();
        assert_eq!(form.bridge.model().error, None, "the press is only queued");
        settle();
        assert_eq!(
            form.bridge.model().error.as_deref(),
            Some("Choose a folder to write the project into")
        );
        // Copied out first: no borrow is held while GTK runs.
        let title = form.bridge.model().form.title.clone();
        title.set_text("  ");
        create.emit_clicked();
        form.bridge
            .send(NewProjectInput::FolderChosen(PathBuf::from("/nowhere")));
        settle();
        assert_eq!(
            form.bridge.model().error.as_deref(),
            Some("Give the project a name"),
            "Create was handled before the folder came"
        );
        assert_eq!(
            form.bridge.model().folder.as_deref(),
            Some(std::path::Path::new("/nowhere"))
        );
        assert_eq!(heard.get(), 0, "nothing was created or closed");
    }
}
