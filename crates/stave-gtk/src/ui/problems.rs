//! The Problems sidebar (Problems.dc.html), opened by the status pill:
//! its title and close button, the All / Errors / Warnings filters, and the
//! parts tree showing only the parts with problems, each problem kind one
//! compact row (`<icon> Motion (2)  project.stave[69], …`). The rows and
//! their grouping come from `stave_app::problems::tree`; choosing a row
//! selects its first problem.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::check::Checked;
use stave_app::inspect::SwatchShape;
use stave_app::problems::{self, Filter, ProblemAssembly, ProblemEntry, ProblemRow, ProblemsTree};
use stave_app::session::{ProblemIndex, Selection};
use stave_lang::Severity;

use super::icons::{self, PathIcon};

/// Called with the problem a row stands for.
type ProblemCallback = Rc<dyn Fn(ProblemIndex)>;

#[derive(Default)]
struct State {
    checked: Option<Rc<Checked>>,
    filter: Filter,
    selection: Option<Selection>,
    on_problem: Option<ProblemCallback>,
}

/// The Problems sidebar.
pub struct ProblemsSidebar {
    /// The sidebar page.
    pub root: gtk::Box,
    /// Closes the list (back to the parts tree).
    pub close: gtk::Button,
    all: gtk::ToggleButton,
    errors: gtk::ToggleButton,
    warnings: gtk::ToggleButton,
    lead: gtk::Label,
    list: gtk::Box,
    state: Rc<RefCell<State>>,
}

fn filter_button(class: &str) -> gtk::ToggleButton {
    gtk::ToggleButton::builder()
        .css_classes(["stave-filter", class])
        .build()
}

impl ProblemsSidebar {
    /// An empty sidebar.
    pub fn new() -> ProblemsSidebar {
        let title = gtk::Label::builder()
            .label("Problems")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["stave-problems-title"])
            .build();
        let close = gtk::Button::builder()
            .child(&PathIcon::new(icons::CLOSE, 12))
            .tooltip_text("Close the problems list")
            .css_classes(["flat", "circular", "stave-close"])
            .build();
        close.update_property(&[gtk::accessible::Property::Label("Close the problems list")]);
        let head = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-problems-head"])
            .build();
        head.append(&title);
        head.append(&close);

        let all = filter_button("all");
        let errors = filter_button("errors");
        let warnings = filter_button("warnings");
        errors.set_group(Some(&all));
        warnings.set_group(Some(&all));
        all.set_active(true);
        let filters = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .css_classes(["stave-filters"])
            .build();
        filters.update_property(&[gtk::accessible::Property::Label("Show")]);
        filters.append(&all);
        filters.append(&errors);
        filters.append(&warnings);

        let lead = gtk::Label::builder()
            .label("The parts tree, showing only parts with problems")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["stave-problems-lead"])
            .build();
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .css_classes(["stave-problems-tree"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Parts with problems")]);
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        let about = gtk::Label::builder()
            .use_markup(true)
            .label("<b>What Stave checks here</b>\nWood passing through wood, joint parts that don’t meet, slide clearance, every door and drawer through its whole travel, and that each joint can be put together in the order its insert steps say.")
            .xalign(0.0)
            .wrap(true)
            .css_classes(["stave-problems-about"])
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["stave-sidebar", "stave-problems"])
            .width_request(240)
            .build();
        root.append(&head);
        root.append(&filters);
        root.append(&lead);
        root.append(&scrolled);
        root.append(&about);
        let sidebar = ProblemsSidebar {
            root,
            close,
            all,
            errors,
            warnings,
            lead,
            list,
            state: Rc::new(RefCell::new(State::default())),
        };
        sidebar.connect();
        sidebar.update_counts();
        sidebar
    }

    fn connect(&self) {
        for (button, filter) in [
            (&self.all, Filter::All),
            (&self.errors, Filter::Errors),
            (&self.warnings, Filter::Warnings),
        ] {
            let state = self.state.clone();
            let list = self.list.clone();
            let lead = self.lead.clone();
            button.connect_toggled(move |button| {
                if !button.is_active() {
                    return;
                }
                state.borrow_mut().filter = filter;
                fill(&list, &lead, &state);
            });
        }
    }

    /// Calls `callback` with a problem when its row is chosen.
    pub fn connect_problem(&self, callback: impl Fn(ProblemIndex) + 'static) {
        self.state.borrow_mut().on_problem = Some(Rc::new(callback));
    }

    /// Lists a check's problems (`None`: not checked yet).
    pub fn show(&self, checked: Option<Rc<Checked>>) {
        self.state.borrow_mut().checked = checked;
        self.update_counts();
        fill(&self.list, &self.lead, &self.state);
    }

    /// Marks the selected problem's row.
    pub fn show_selection(&self, selection: Option<&Selection>) {
        self.state.borrow_mut().selection = selection.cloned();
        fill(&self.list, &self.lead, &self.state);
    }

    fn update_counts(&self) {
        let counts = {
            let state = self.state.borrow();
            state
                .checked
                .as_ref()
                .map(|checked| problems::tree(checked, Filter::All).counts)
                .unwrap_or_default()
        };
        self.all.set_label(&format!("All {}", counts.all));
        self.errors.set_label(&format!("Errors {}", counts.errors));
        self.warnings
            .set_label(&format!("Warnings {}", counts.warnings));
    }
}

/// Rebuilds the rows.
fn fill(list: &gtk::Box, lead: &gtk::Label, state: &Rc<RefCell<State>>) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    let (tree, selection) = {
        let state = state.borrow();
        let tree = state
            .checked
            .as_ref()
            .map(|checked| problems::tree(checked, state.filter));
        (tree, state.selection.clone())
    };
    let Some(tree) = tree else {
        lead.set_visible(false);
        list.append(&empty("Checking…", ""));
        return;
    };
    let has_tree = !tree.carcass.is_empty() || !tree.assemblies.is_empty();
    lead.set_visible(has_tree);
    if tree.is_empty() {
        let (title, words) = if tree.counts.all == 0 {
            ("No problems", "Everything Stave checks holds.")
        } else {
            ("None of these", "Nothing of this kind to show.")
        };
        list.append(&empty(title, words));
        return;
    }
    let context = Context {
        state: state.clone(),
        selection,
    };
    if !tree.carcass.is_empty() {
        list.append(&heading("CARCASS", true));
        for entry in &tree.carcass {
            entry_rows(list, &context, entry, 0);
        }
    }
    if !tree.assemblies.is_empty() {
        list.append(&heading("ASSEMBLIES", tree.carcass.is_empty()));
        for node in &tree.assemblies {
            assembly_rows(list, &context, node, 0);
        }
    }
    elsewhere_rows(list, &context, &tree);
}

/// What every row needs to know.
struct Context {
    state: Rc<RefCell<State>>,
    selection: Option<Selection>,
}

fn empty(title: &str, words: &str) -> gtk::Box {
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .margin_top(24)
        .build();
    root.append(
        &gtk::Label::builder()
            .label(title)
            .css_classes(["heading"])
            .build(),
    );
    if !words.is_empty() {
        root.append(
            &gtk::Label::builder()
                .label(words)
                .wrap(true)
                .justify(gtk::Justification::Center)
                .css_classes(["dim-label"])
                .build(),
        );
    }
    root
}

fn heading(text: &str, first: bool) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["stave-heading"])
        .build();
    if first {
        label.add_css_class("first");
    }
    label
}

fn indent_class(depth: usize) -> &'static str {
    match depth {
        0 => "depth0",
        1 => "depth1",
        _ => "depth2",
    }
}

fn assembly_rows(list: &gtk::Box, context: &Context, node: &ProblemAssembly, depth: usize) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .css_classes(["stave-problem-assembly", indent_class(depth)])
        .build();
    content.append(&PathIcon::new(icons::EXPANDED, 12));
    content.append(
        &gtk::Label::builder()
            .label(&node.label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    if let Some(detail) = &node.detail {
        content.append(
            &gtk::Label::builder()
                .label(detail)
                .css_classes(["stave-detail"])
                .build(),
        );
    }
    content.set_accessible_role(gtk::AccessibleRole::TreeItem);
    content.update_property(&[gtk::accessible::Property::Label(&node.label)]);
    list.append(&content);
    for entry in &node.entries {
        entry_rows(list, context, entry, depth.saturating_add(1));
    }
    for child in &node.children {
        assembly_rows(list, context, child, depth.saturating_add(1));
    }
}

fn entry_rows(list: &gtk::Box, context: &Context, entry: &ProblemEntry, depth: usize) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .css_classes(["stave-problem-part", indent_class(depth)])
        .build();
    let swatch = entry.entry.swatch;
    content.append(&icons::swatch(
        gtk::gdk::RGBA::new(
            f32::from(swatch.colour.red) / 255.0,
            f32::from(swatch.colour.green) / 255.0,
            f32::from(swatch.colour.blue) / 255.0,
            1.0,
        ),
        swatch.shape == SwatchShape::Round,
        10,
    ));
    content.append(
        &gtk::Label::builder()
            .label(&entry.entry.label)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    if entry.involved() {
        content.append(
            &gtk::Label::builder()
                .label("involved")
                .css_classes(["stave-detail"])
                .tooltip_text("Another part's problem involves this one")
                .build(),
        );
    } else {
        let badge = gtk::Label::builder()
            .label(entry.about.to_string())
            .css_classes([
                "stave-badge",
                severity_class(entry.severity.unwrap_or(Severity::Error)),
            ])
            .valign(gtk::Align::Center)
            .build();
        content.append(&badge);
    }
    content.set_accessible_role(gtk::AccessibleRole::TreeItem);
    content.update_property(&[gtk::accessible::Property::Label(&entry.entry.label)]);
    list.append(&content);
    // The selected problem is marked under the part it is about, not
    // under the parts it only involves.
    for row in &entry.rows {
        list.append(&problem_row(context, row, depth, !entry.involved()));
    }
}

fn elsewhere_rows(list: &gtk::Box, context: &Context, tree: &ProblemsTree) {
    if tree.elsewhere.is_empty() {
        return;
    }
    let first = tree.carcass.is_empty() && tree.assemblies.is_empty();
    list.append(&heading(
        if first {
            "IN THE TEXT"
        } else {
            "NOT ABOUT ONE PART"
        },
        first,
    ));
    for row in &tree.elsewhere {
        list.append(&problem_row(context, row, 0, true));
    }
}

fn severity_class(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
    }
}

/// `<icon> Motion (2)  project.stave[69], …` as a button.
fn problem_row(context: &Context, row: &ProblemRow, depth: usize, marks: bool) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let icon = PathIcon::new(icons::category(row.category), 14);
    icon.add_css_class(severity_class(row.severity));
    icon.add_css_class("stave-problem-icon");
    content.append(&icon);
    content.append(
        &gtk::Label::builder()
            .label(row.title())
            .css_classes(["stave-problem-kind"])
            .build(),
    );
    content.append(
        &gtk::Label::builder()
            .label(row.places_text())
            .xalign(1.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["stave-problem-places"])
            .build(),
    );
    let button = gtk::Button::builder()
        .child(&content)
        .tooltip_text(&row.messages)
        .css_classes(["stave-problem-row", indent_class(depth)])
        .build();
    button.set_accessible_role(gtk::AccessibleRole::TreeItem);
    button.update_property(&[gtk::accessible::Property::Label(&format!(
        "{}: {}",
        row.title(),
        row.messages
    ))]);
    if marks && row.holds(context.selection.as_ref()) {
        button.add_css_class("selected");
        button.update_state(&[gtk::accessible::State::Selected(Some(true))]);
    }
    let first = *row.problems.first();
    let state = context.state.clone();
    button.connect_clicked(move |_| {
        let callback = state.borrow().on_problem.clone();
        if let Some(callback) = callback {
            callback(first);
        }
    });
    button
}
