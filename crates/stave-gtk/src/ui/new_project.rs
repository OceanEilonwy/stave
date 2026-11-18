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

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
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

/// The New Project dialog's fields.
pub(super) struct NewProjectDialog {
    /// The dialog.
    pub(super) dialog: adw::Dialog,
    title: adw::EntryRow,
    sizes: [adw::SpinRow; 3],
    stock_name: adw::EntryRow,
    species: adw::ComboRow,
    form: adw::ComboRow,
    thickness: adw::SpinRow,
    max_length: adw::SpinRow,
    libraries: Vec<(&'static str, gtk::CheckButton)>,
    tools: Vec<(ToolChoice, gtk::CheckButton)>,
    folder_label: gtk::Label,
    folder: RefCell<Option<PathBuf>>,
    folder_button: gtk::Button,
    create: gtk::Button,
    error: gtk::Label,
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

impl NewProjectDialog {
    /// The dialog, filled with a small table's sizes and oak.
    pub(super) fn new() -> Rc<NewProjectDialog> {
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

        let folder_label = gtk::Label::builder()
            .label("No folder chosen yet")
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .css_classes(["dim-label"])
            .build();
        let folder_button = gtk::Button::builder().label("Choose Folder…").build();
        let folder_row = adw::ActionRow::builder().title("Write it into").build();
        folder_row.add_suffix(&folder_label);
        folder_row.add_suffix(&folder_button);
        let place = group("Folder", Some("The project's .stave file is written here"));
        place.add(&folder_row);

        let error = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["error"])
            .visible(false)
            .build();
        let page = adw::PreferencesPage::new();
        for group in [
            &general,
            &size,
            &wood,
            &libraries_group,
            &tools_group,
            &place,
        ] {
            page.add(group);
        }
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&page);
        error.set_margin_start(24);
        error.set_margin_end(24);
        error.set_margin_bottom(12);
        body.append(&error);

        let cancel = gtk::Button::builder().label("Cancel").build();
        let create = gtk::Button::builder()
            .label("Create")
            .css_classes(["suggested-action"])
            .build();
        let header = adw::HeaderBar::builder()
            .show_start_title_buttons(false)
            .show_end_title_buttons(false)
            .build();
        header.pack_start(&cancel);
        header.pack_end(&create);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&body));
        let dialog = adw::Dialog::builder()
            .title("New Project")
            .content_width(560)
            .content_height(720)
            .child(&view)
            .build();
        {
            let dialog = dialog.downgrade();
            cancel.connect_clicked(move |_| {
                if let Some(dialog) = dialog.upgrade() {
                    dialog.close();
                }
            });
        }
        Rc::new(NewProjectDialog {
            dialog,
            title,
            sizes,
            stock_name,
            species,
            form,
            thickness,
            max_length,
            libraries,
            tools,
            folder_label,
            folder: RefCell::new(None),
            folder_button,
            create,
            error,
        })
    }

    /// What the fields hold.
    pub(super) fn answers(&self) -> Answers {
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

    /// Says what is wrong under the fields; `None` hides it.
    fn show_error(&self, words: Option<&str>) {
        self.error.set_label(words.unwrap_or_default());
        self.error.set_visible(words.is_some());
    }

    /// The folder the project is written into.
    fn set_folder(&self, folder: PathBuf) {
        self.folder_label.set_label(&folder.display().to_string());
        self.folder_label.remove_css_class("dim-label");
        self.folder.replace(Some(folder));
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
    /// Asks for a new project and, once it is written, opens it.
    pub(super) fn ask_new_project(self: &Rc<Self>) {
        let form = NewProjectDialog::new();
        {
            let form_weak = Rc::downgrade(&form);
            let window = self.widgets.window.clone();
            form.folder_button.connect_clicked(move |_| {
                let chooser = gtk::FileDialog::builder()
                    .title("Choose the folder for the new project")
                    .modal(true)
                    .build();
                let form = form_weak.clone();
                chooser.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
                    if let (Ok(folder), Some(form)) = (result, form.upgrade())
                        && let Some(path) = folder.path()
                    {
                        form.set_folder(path);
                    }
                });
            });
        }
        {
            let form_weak = Rc::downgrade(&form);
            let window = self.clone();
            form.create.connect_clicked(move |_| {
                let Some(form) = form_weak.upgrade() else {
                    return;
                };
                let project = match form.answers().project() {
                    Ok(project) => project,
                    Err(words) => return form.show_error(Some(&words)),
                };
                let Some(folder) = form.folder.borrow().clone() else {
                    return form.show_error(Some("Choose a folder to write the project into"));
                };
                match create_project(&folder, &project) {
                    Ok(_) => {
                        form.show_error(None);
                        form.dialog.close();
                        let next = window.clone();
                        window.ask_about_unsaved(move || {
                            next.open_folder(folder);
                            next.light_first_space(ENVELOPE_REGION);
                            next.show_hint("Click inside the box to add the first piece");
                        });
                    }
                    Err(words) => form.show_error(Some(&words)),
                }
            });
        }
        // The dialog keeps its fields alive while it shows.
        let keep = RefCell::new(Some(form.clone()));
        form.dialog.connect_closed(move |_| {
            keep.borrow_mut().take();
        });
        form.dialog.present(Some(&self.widgets.window));
    }
}
