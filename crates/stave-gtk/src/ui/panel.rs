//! The side panel: the selected part or hardware, and the project's
//! problems.

use std::cell::RefCell;

use adw::prelude::*;
use stave_app::view::{Description, Problem};
use stave_lang::Severity;

/// The side panel's widgets.
pub struct Panel {
    /// The panel, ready to put in the split view.
    pub root: gtk::ScrolledWindow,
    selection: adw::PreferencesGroup,
    selection_rows: RefCell<Vec<gtk::Widget>>,
    problems: adw::PreferencesGroup,
    problem_rows: RefCell<Vec<gtk::Widget>>,
}

fn property_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .use_markup(false)
        .subtitle_selectable(true)
        .build();
    row.add_css_class("property");
    row
}

impl Panel {
    /// An empty panel.
    pub fn new() -> Panel {
        let selection = adw::PreferencesGroup::builder()
            .title("Selection")
            .description("Click a part to see its stock, species and size.")
            .build();
        let problems = adw::PreferencesGroup::builder()
            .title("Problems")
            .visible(false)
            .build();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(24)
            .margin_top(12)
            .margin_bottom(18)
            .margin_start(12)
            .margin_end(12)
            .build();
        content.append(&problems);
        content.append(&selection);
        content.append(&keys_group());
        let clamp = adw::Clamp::builder()
            .maximum_size(420)
            .child(&content)
            .build();
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .build();
        Panel {
            root,
            selection,
            selection_rows: RefCell::new(Vec::new()),
            problems,
            problem_rows: RefCell::new(Vec::new()),
        }
    }

    /// Shows the selected item, or the hint when nothing is selected.
    pub fn show_selection(&self, description: Option<&Description>) {
        for row in self.selection_rows.take() {
            self.selection.remove(&row);
        }
        let Some(description) = description else {
            self.selection.set_title("Selection");
            self.selection
                .set_description(Some("Click a part to see its stock, species and size."));
            return;
        };
        self.selection.set_title(description.kind);
        self.selection.set_description(None);
        let mut rows = Vec::with_capacity(description.rows.len() + 1);
        let name = property_row("Name", &description.name);
        self.selection.add(&name);
        rows.push(name.upcast());
        for entry in &description.rows {
            let row = property_row(entry.label, &entry.value);
            self.selection.add(&row);
            rows.push(row.upcast());
        }
        self.selection_rows.replace(rows);
    }

    /// Lists the project's problems; hides the group when there are none.
    pub fn show_problems(&self, problems: &[Problem]) {
        for row in self.problem_rows.take() {
            self.problems.remove(&row);
        }
        self.problems.set_visible(!problems.is_empty());
        let errors = problems
            .iter()
            .filter(|p| p.severity == Severity::Error)
            .count();
        let warnings = problems.len() - errors;
        let mut counts = Vec::new();
        if errors > 0 {
            counts.push(format!(
                "{errors} {}",
                if errors == 1 { "error" } else { "errors" }
            ));
        }
        if warnings > 0 {
            counts.push(format!(
                "{warnings} {}",
                if warnings == 1 { "warning" } else { "warnings" }
            ));
        }
        self.problems
            .set_title(&format!("Problems ({})", counts.join(", ")));
        let mut rows = Vec::with_capacity(problems.len());
        for problem in problems {
            let (icon, style, word) = match problem.severity {
                Severity::Error => ("dialog-error-symbolic", "error", "error"),
                Severity::Warning => ("dialog-warning-symbolic", "warning", "warning"),
                Severity::Note => ("dialog-information-symbolic", "dim-label", "note"),
            };
            let mut subtitle = format!("{word}[{}]", problem.code);
            if let Some(location) = &problem.location {
                subtitle.push_str(" at ");
                subtitle.push_str(location);
            }
            if let Some(help) = &problem.help {
                subtitle.push_str("\nhelp: ");
                subtitle.push_str(help);
            }
            let row = adw::ActionRow::builder()
                .title(&problem.message)
                .subtitle(&subtitle)
                .use_markup(false)
                .title_lines(0)
                .subtitle_lines(0)
                .subtitle_selectable(true)
                .build();
            let image = gtk::Image::from_icon_name(icon);
            image.add_css_class(style);
            row.add_prefix(&image);
            self.problems.add(&row);
            rows.push(row.upcast());
        }
        self.problem_rows.replace(rows);
    }
}

fn keys_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Mouse and keys")
        .build();
    for (what, how) in [
        ("Turn", "drag with the left button"),
        ("Move", "drag with the right or middle button"),
        ("Zoom", "scroll"),
        ("Select", "click a part"),
        ("Frame", "F: the selection, or everything"),
        ("Clear selection", "Escape"),
    ] {
        let row = adw::ActionRow::builder()
            .title(what)
            .subtitle(how)
            .use_markup(false)
            .build();
        group.add(&row);
    }
    group
}
