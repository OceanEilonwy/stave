//! The side panel: the selected part or hardware, the project's problems
//! (a problem the checks can show is a button that poses the design where
//! it happens), the shown problem's motion slider, and the explode control.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::view::{Description, Problem};
use stave_lang::Severity;

/// Called with a problem's index when its row is activated.
type ProblemCallback = Rc<dyn Fn(usize)>;

/// A shown problem's motion slider: its full range, where it starts, and
/// its unit ("degrees" or "millimetres").
#[derive(Debug, Clone, Copy)]
pub struct MotionSlider<'a> {
    /// Fully open.
    pub full: f64,
    /// Where the slider starts.
    pub value: f64,
    /// What the numbers are.
    pub unit: &'a str,
}

/// The side panel's widgets.
pub struct Panel {
    /// The panel, ready to put in the split view.
    pub root: gtk::ScrolledWindow,
    selection: adw::PreferencesGroup,
    selection_rows: RefCell<Vec<gtk::Widget>>,
    problems: adw::PreferencesGroup,
    problem_rows: RefCell<Vec<gtk::Widget>>,
    /// Called with a problem's index when its row is activated.
    on_problem: RefCell<Option<ProblemCallback>>,
    /// The problem being shown: its message, motion slider and close button.
    shown: adw::PreferencesGroup,
    shown_message: gtk::Label,
    /// Scrubs the shown problem's motion (degrees or millimetres).
    pub motion: gtk::Scale,
    motion_row: adw::PreferencesRow,
    /// Stops showing the problem.
    pub close: gtk::Button,
    /// Opens the selected item's door or drawer (degrees or millimetres).
    pub opening: gtk::Scale,
    opening_group: adw::PreferencesGroup,
    opening_label: gtk::Label,
    /// How far apart the parts are pulled, 0 to 1.
    pub explode: gtk::Scale,
    /// Eases the explode control to fully apart, or back together.
    pub animate: gtk::ToggleButton,
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
        let shown = adw::PreferencesGroup::builder()
            .title("Shown problem")
            .visible(false)
            .build();
        let shown_message = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .selectable(true)
            .build();
        let close = gtk::Button::builder()
            .label("Done")
            .tooltip_text("Stop showing the problem")
            .valign(gtk::Align::Center)
            .build();
        shown.set_header_suffix(Some(&close));
        shown.add(&shown_message);
        let motion = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        motion.set_hexpand(true);
        motion.set_draw_value(true);
        motion.set_value_pos(gtk::PositionType::Right);
        let motion_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        motion_box.set_margin_top(8);
        motion_box.set_margin_bottom(8);
        motion_box.set_margin_start(12);
        motion_box.set_margin_end(12);
        motion_box.append(
            &gtk::Label::builder()
                .label("Move it: closed to fully open")
                .xalign(0.0)
                .css_classes(["dim-label"])
                .build(),
        );
        motion_box.append(&motion);
        let motion_row = adw::PreferencesRow::builder()
            .child(&motion_box)
            .activatable(false)
            .build();
        shown.add(&motion_row);

        let view = adw::PreferencesGroup::builder()
            .title("Exploded view")
            .build();
        let explode = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.01);
        explode.set_hexpand(true);
        explode.set_draw_value(false);
        let animate = gtk::ToggleButton::builder()
            .label("Pull apart")
            .tooltip_text("Pull the parts apart along their joints, or put them back")
            .valign(gtk::Align::Center)
            .build();
        let explode_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        explode_box.set_margin_top(8);
        explode_box.set_margin_bottom(8);
        explode_box.set_margin_start(12);
        explode_box.set_margin_end(12);
        explode_box.append(&explode);
        explode_box.append(&animate);
        view.add(
            &adw::PreferencesRow::builder()
                .child(&explode_box)
                .activatable(false)
                .build(),
        );

        let opening_group = adw::PreferencesGroup::builder()
            .title("Open it")
            .visible(false)
            .build();
        let opening = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        opening.set_hexpand(true);
        opening.set_draw_value(true);
        opening.set_value_pos(gtk::PositionType::Right);
        let opening_label = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .css_classes(["dim-label"])
            .build();
        let opening_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        opening_box.set_margin_top(8);
        opening_box.set_margin_bottom(8);
        opening_box.set_margin_start(12);
        opening_box.set_margin_end(12);
        opening_box.append(&opening_label);
        opening_box.append(&opening);
        opening_group.add(
            &adw::PreferencesRow::builder()
                .child(&opening_box)
                .activatable(false)
                .build(),
        );

        content.append(&shown);
        content.append(&problems);
        content.append(&view);
        content.append(&selection);
        content.append(&opening_group);
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
            on_problem: RefCell::new(None),
            shown,
            shown_message,
            motion,
            motion_row,
            close,
            opening,
            opening_group,
            opening_label,
            explode,
            animate,
        }
    }

    /// Shows a slider that opens the selected item's door or drawer, named
    /// `what`, from closed to `full` (degrees or millimetres); `None`
    /// hides it.
    pub fn show_opening(&self, what: Option<(&str, MotionSlider<'_>)>) {
        let Some((name, slider)) = what else {
            self.opening_group.set_visible(false);
            return;
        };
        self.opening_group.set_visible(true);
        self.opening_label
            .set_label(&format!("{name}: closed to fully open, in {}", slider.unit));
        self.opening.set_range(0.0, slider.full.max(0.1));
        self.opening.set_value(slider.value);
        self.opening.set_tooltip_text(Some(slider.unit));
    }

    /// Calls `callback` with a problem's index when its row is activated.
    pub fn connect_problem(&self, callback: impl Fn(usize) + 'static) {
        self.on_problem.replace(Some(Rc::new(callback)));
    }

    /// Shows which problem is posed, with a motion slider from 0 to `full`
    /// (degrees or millimetres) at `value` when it has a motion; `None`
    /// hides the group.
    pub fn show_problem(&self, message: Option<&str>, motion: Option<MotionSlider<'_>>) {
        let Some(message) = message else {
            self.shown.set_visible(false);
            return;
        };
        self.shown.set_visible(true);
        self.shown_message.set_label(message);
        match motion {
            Some(slider) => {
                self.motion_row.set_visible(true);
                self.motion.set_range(0.0, slider.full.max(0.1));
                self.motion.set_value(slider.value);
                self.motion.set_tooltip_text(Some(slider.unit));
            }
            None => self.motion_row.set_visible(false),
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
            if problem.contact.is_some() {
                row.set_activatable(true);
                row.set_tooltip_text(Some("Show where it happens"));
                row.add_suffix(&gtk::Image::from_icon_name("find-location-symbolic"));
                let callback = self.on_problem.borrow().clone();
                let index = rows.len();
                row.connect_activated(move |_| {
                    if let Some(callback) = &callback {
                        callback(index);
                    }
                });
            }
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
        ("Show a problem", "click it in the list"),
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
