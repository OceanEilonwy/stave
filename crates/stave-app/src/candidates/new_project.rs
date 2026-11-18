//! The text of a new project started from a box (`docs/editing.md`,
//! "Naming rules", Envelope): the `project` item, the three `overall_`
//! params, `region envelope = box(...)`, the stock and the tools.
//!
//! ```text
//! stave 1
//!
//! project "Hall table" {
//!   units: mm
//!   tools: [saw, chisel]
//! }
//!
//! param overall_width  = 900mm
//! param overall_height = 760mm
//! param overall_depth  = 400mm
//!
//! region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)
//!
//! stock oak_19 { species: white_oak, form: solid, thickness: 19mm, max_length: 2400mm }
//! ```
//!
//! The text is laid out as `stave fmt` lays it out.

use stave_lang::ast::Ident;
use stave_lang::units::Length;
use stave_lang::{FileId, SourceMap};
use stave_schema::species::Species;
use stave_schema::words::StockForm;

use super::print::quoted;
use crate::naming::{ENVELOPE_PARAMS, ENVELOPE_REGION};

/// What a new project is started from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProject {
    /// The project's title: `Hall table`.
    pub title: String,
    /// The overall width, height and depth, across, up and deep.
    pub size: [Length; 3],
    /// The stock to buy.
    pub stock: Vec<NewStock>,
    /// The tools the maker has, by their kind's name; empty means every
    /// tool is assumed, and the `tools:` field is left out.
    pub tools: Vec<Ident>,
}

/// One stock item of a new project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewStock {
    /// Its name: `oak_19`.
    pub name: Ident,
    /// The wood.
    pub species: Species,
    /// How it is bought, with its sizes.
    pub cut: StockCut,
}

/// How a stock is bought, with the sizes its form needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StockCut {
    /// Boards or sheets.
    Board {
        /// Solid boards, plywood or MDF.
        form: BoardForm,
        /// The finished thickness.
        thickness: Length,
        /// A fixed width, for a blank or a strip.
        width: Option<Length>,
        /// The widest board you can get.
        max_width: Option<Length>,
        /// The longest board or sheet you can get.
        max_length: Length,
    },
    /// Round stock.
    Dowel {
        /// Its diameter.
        diameter: Length,
    },
}

/// The forms of stock bought flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoardForm {
    /// Solid wood boards.
    Solid,
    /// Plywood sheets.
    Plywood,
    /// MDF sheets.
    Mdf,
}

impl BoardForm {
    fn form(self) -> StockForm {
        match self {
            BoardForm::Solid => StockForm::Solid,
            BoardForm::Plywood => StockForm::Plywood,
            BoardForm::Mdf => StockForm::Mdf,
        }
    }
}

/// Why a new project's text can't be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NewProjectError {
    /// An overall size of zero or less.
    #[error("the overall {0} must be more than 0mm")]
    SizeNotPositive(&'static str),
    /// A stock size of zero or less.
    #[error("{stock}'s {field} must be more than 0mm")]
    StockSizeNotPositive {
        /// The stock.
        stock: String,
        /// The field.
        field: &'static str,
    },
    /// Two stock items, or a stock and an envelope name, share a name.
    #[error("the name '{0}' is used twice")]
    DuplicateName(String),
    /// The written text didn't parse: a bug in the writer.
    #[error("the new project's text doesn't parse; please report this as a bug")]
    Unparsable,
}

/// The text of a new project. It parses and checks: the envelope `box` is
/// its root.
///
/// # Errors
///
/// A size of zero or less, a name used twice; see [`NewProjectError`].
pub fn new_project_text(project: &NewProject) -> Result<String, NewProjectError> {
    let directions = ["width", "height", "depth"];
    for (size, direction) in project.size.iter().zip(directions) {
        if *size <= Length::ZERO {
            return Err(NewProjectError::SizeNotPositive(direction));
        }
    }
    let mut names: Vec<&str> = ENVELOPE_PARAMS.to_vec();
    names.push(ENVELOPE_REGION);
    for stock in &project.stock {
        if names.contains(&stock.name.as_str()) {
            return Err(NewProjectError::DuplicateName(
                stock.name.as_str().to_owned(),
            ));
        }
        names.push(stock.name.as_str());
    }

    let mut text = String::from("stave 1\n\n");
    text.push_str(&format!("project {} {{\n", quoted(&project.title)));
    text.push_str("  units: mm\n");
    if !project.tools.is_empty() {
        let tools: Vec<&str> = project.tools.iter().map(Ident::as_str).collect();
        text.push_str(&format!("  tools: [{}]\n", tools.join(", ")));
    }
    text.push_str("}\n\n");
    for (name, size) in ENVELOPE_PARAMS.iter().zip(project.size) {
        text.push_str(&format!("param {name} = {size}\n"));
    }
    let [across, up, deep] = ENVELOPE_PARAMS;
    text.push_str(&format!(
        "\nregion {ENVELOPE_REGION} = box(across: {across}, up: {up}, deep: {deep})\n"
    ));
    if !project.stock.is_empty() {
        text.push('\n');
    }
    for stock in &project.stock {
        text.push_str(&stock_line(stock)?);
        text.push('\n');
    }

    let mut sources = SourceMap::new();
    let file: FileId = sources
        .add("project.stave", text.as_str())
        .map_err(|_| NewProjectError::Unparsable)?;
    if stave_lang::parse_file(file, &text).has_errors() {
        return Err(NewProjectError::Unparsable);
    }
    Ok(stave_lang::format_file(file, &text).unwrap_or(text))
}

fn stock_line(stock: &NewStock) -> Result<String, NewProjectError> {
    let name = stock.name.as_str();
    let positive = |field: &'static str, size: Length| {
        if size > Length::ZERO {
            Ok(size)
        } else {
            Err(NewProjectError::StockSizeNotPositive {
                stock: name.to_owned(),
                field,
            })
        }
    };
    let mut fields = vec![format!("species: {}", stock.species.word())];
    match stock.cut {
        StockCut::Board {
            form,
            thickness,
            width,
            max_width,
            max_length,
        } => {
            fields.push(format!("form: {}", form.form().word()));
            fields.push(format!("thickness: {}", positive("thickness", thickness)?));
            if let Some(width) = width {
                fields.push(format!("width: {}", positive("width", width)?));
            }
            if let Some(max_width) = max_width {
                fields.push(format!("max_width: {}", positive("max_width", max_width)?));
            }
            fields.push(format!(
                "max_length: {}",
                positive("max_length", max_length)?
            ));
        }
        StockCut::Dowel { diameter } => {
            fields.push(format!("form: {}", StockForm::Dowel.word()));
            fields.push(format!("diameter: {}", positive("diameter", diameter)?));
        }
    }
    Ok(format!("stock {name} {{ {} }}", fields.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::ast::{DesignItem, ItemKind, Span};

    fn name(text: &str) -> Ident {
        Ident::new(text, Span::default()).unwrap()
    }

    fn table() -> NewProject {
        NewProject {
            title: "Hall \"table\"".to_owned(),
            size: [
                Length::from_mm(900),
                Length::from_mm(760),
                Length::from_nm(400_500_000),
            ],
            stock: vec![
                NewStock {
                    name: name("oak_19"),
                    species: Species::WhiteOak,
                    cut: StockCut::Board {
                        form: BoardForm::Solid,
                        thickness: Length::from_mm(19),
                        width: None,
                        max_width: Some(Length::from_mm(200)),
                        max_length: Length::from_mm(2400),
                    },
                },
                NewStock {
                    name: name("ply_6"),
                    species: Species::Birch,
                    cut: StockCut::Board {
                        form: BoardForm::Plywood,
                        thickness: Length::from_mm(6),
                        width: None,
                        max_width: None,
                        max_length: Length::from_mm(2440),
                    },
                },
                NewStock {
                    name: name("dowel_8"),
                    species: Species::Beech,
                    cut: StockCut::Dowel {
                        diameter: Length::from_mm(8),
                    },
                },
            ],
            tools: vec![name("saw"), name("chisel")],
        }
    }

    #[test]
    fn a_new_project_parses_with_its_envelope_stock_and_tools() {
        let text = new_project_text(&table()).unwrap();
        let mut sources = SourceMap::new();
        let file = sources.add("project.stave", text.as_str()).unwrap();
        let parse = stave_lang::parse_file(file, &text);
        assert!(!parse.has_errors(), "{text}\n{:?}", parse.diagnostics);
        let items = parse.ast.unwrap().items;
        let names: Vec<String> = items
            .iter()
            .filter_map(|item| item.kind.name().map(|n| n.as_str().to_owned()))
            .collect();
        assert_eq!(
            names,
            [
                "overall_width",
                "overall_height",
                "overall_depth",
                "envelope",
                "oak_19",
                "ply_6",
                "dowel_8"
            ]
        );
        assert!(matches!(
            items.iter().find(|i| i.kind.name().is_some_and(|n| n.as_str() == "envelope")),
            Some(item) if matches!(item.kind, ItemKind::Design(DesignItem::Region(_)))
        ));
        assert!(text.starts_with("stave 1\n\nproject \"Hall \\\"table\\\"\" {\n"));
        assert!(text.contains("tools: [saw, chisel]"), "{text}");
        assert!(text.contains("param overall_depth  = 400.5mm"), "{text}");
        assert!(text.contains(
            "region envelope = box(across: overall_width, up: overall_height, deep: overall_depth)"
        ));
        assert!(text.contains(
            "{ species: white_oak, form: solid, thickness: 19mm, max_width: 200mm, max_length: 2400mm }"
        ), "{text}");
        assert!(
            text.contains("stock dowel_8 { species: beech, form: dowel, diameter: 8mm }"),
            "{text}"
        );
    }

    #[test]
    fn a_new_project_checks_with_the_box_as_its_root() {
        // Tools the prelude declares: the table's `saw` is only a name.
        let mut project = table();
        project.tools = vec![name("tenon_saw"), name("chisel")];
        let text = new_project_text(&project).unwrap();
        let folder = std::path::PathBuf::from("/test/table");
        let files = stave_modules::MemoryFiles::new().with(folder.join("project.stave"), &text);
        let opened = crate::open_with(&folder, &files).unwrap();
        let errors: Vec<_> = opened
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .collect();
        assert!(errors.is_empty(), "{text}\n{errors:?}");
    }

    #[test]
    fn no_tools_leaves_the_field_out() {
        let mut project = table();
        project.tools.clear();
        project.stock.clear();
        let text = new_project_text(&project).unwrap();
        assert!(!text.contains("tools"), "{text}");
    }

    #[test]
    fn sizes_and_names_are_checked() {
        let mut project = table();
        project.size[1] = Length::ZERO;
        assert_eq!(
            new_project_text(&project),
            Err(NewProjectError::SizeNotPositive("height"))
        );
        let mut project = table();
        project.stock[1].name = name("oak_19");
        assert_eq!(
            new_project_text(&project),
            Err(NewProjectError::DuplicateName("oak_19".to_owned()))
        );
        let mut project = table();
        project.stock[2].cut = StockCut::Dowel {
            diameter: Length::ZERO,
        };
        assert!(matches!(
            new_project_text(&project),
            Err(NewProjectError::StockSizeNotPositive {
                field: "diameter",
                ..
            })
        ));
    }
}
