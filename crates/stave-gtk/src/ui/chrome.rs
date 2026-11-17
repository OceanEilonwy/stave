//! The smaller pieces of the window: the status bar, the floating motion
//! bar at the bottom of the 3D view ("Open it", a problem's swing), the
//! problems list that the status pill opens (the slot lane 3 fills), and
//! the placeholder pages of the Cut list and Files views.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::session::{CheckStatus, Tone};
use stave_app::view::Problem;
use stave_lang::Severity;

use super::style;

/// The status bar along the bottom of the window.
pub struct StatusBar {
    /// The bar.
    pub root: gtk::Box,
    dot: gtk::DrawingArea,
    dot_colour: Rc<RefCell<gtk::gdk::RGBA>>,
    spinner: adw::Spinner,
    check: gtk::Label,
    watching: gtk::Label,
    selected: gtk::Label,
}

impl StatusBar {
    /// An empty status bar.
    pub fn new() -> StatusBar {
        let dot_colour = Rc::new(RefCell::new(style::status_dot(Tone::Unknown)));
        let dot = gtk::DrawingArea::builder()
            .content_width(8)
            .content_height(8)
            .valign(gtk::Align::Center)
            .build();
        let colour = dot_colour.clone();
        dot.set_draw_func(move |_, context, width, height| {
            let colour = *colour.borrow();
            let (w, h) = (f64::from(width), f64::from(height));
            context.arc(w / 2.0, h / 2.0, w.min(h) / 2.0, 0.0, std::f64::consts::TAU);
            context.set_source_rgba(
                f64::from(colour.red()),
                f64::from(colour.green()),
                f64::from(colour.blue()),
                1.0,
            );
            let _ = context.fill();
        });
        let spinner = adw::Spinner::builder()
            .width_request(10)
            .height_request(10)
            .visible(false)
            .valign(gtk::Align::Center)
            .build();
        let check = gtk::Label::builder()
            .label("Not checked yet")
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let lead = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        lead.append(&dot);
        lead.append(&spinner);
        lead.append(&check);
        let watching = gtk::Label::new(None);
        let selected = gtk::Label::builder()
            .hexpand(true)
            .xalign(1.0)
            .ellipsize(gtk::pango::EllipsizeMode::Start)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-statusbar"])
            .build();
        root.append(&lead);
        root.append(&watching);
        root.append(&selected);
        StatusBar {
            root,
            dot,
            dot_colour,
            spinner,
            check,
            watching,
            selected,
        }
    }

    /// The check's line and dot.
    pub fn show_check(&self, status: &CheckStatus) {
        self.check.set_label(&status.line());
        let checking = status.is_checking();
        self.spinner.set_visible(checking);
        self.dot.set_visible(!checking);
        *self.dot_colour.borrow_mut() = style::status_dot(status.pill().tone);
        self.dot.queue_draw();
    }

    /// Whether the folder is watched.
    pub fn show_watching(&self, text: &str) {
        self.watching.set_label(text);
    }

    /// "Selected: door_l.stile[hinge]", or what the window is doing.
    pub fn show_selected(&self, text: &str) {
        self.selected.set_label(text);
    }
}

/// The floating bar at the bottom of the 3D view: a name, words at each
/// end and a slider ("door_l  closed ——— 110°").
pub struct MotionBar {
    /// The bar, to overlay on the view.
    pub root: gtk::Box,
    name: gtk::Label,
    start: gtk::Label,
    end: gtk::Label,
    /// The slider.
    pub scale: gtk::Scale,
}

impl MotionBar {
    /// A hidden bar.
    pub fn new() -> MotionBar {
        let name = gtk::Label::builder().css_classes(["heading"]).build();
        let start = gtk::Label::builder().css_classes(["stave-key"]).build();
        let end = gtk::Label::builder().css_classes(["stave-key"]).build();
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        scale.set_width_request(220);
        scale.set_draw_value(false);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .css_classes(["stave-floating"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(14)
            .visible(false)
            .build();
        root.append(&name);
        root.append(&start);
        root.append(&scale);
        root.append(&end);
        MotionBar {
            root,
            name,
            start,
            end,
            scale,
        }
    }

    /// Shows the bar for `name`, from `start` to `end` words, the slider
    /// from 0 to `full` at `value`; `None` hides it.
    pub fn show(&self, what: Option<(&str, &str, &str, f64, f64)>) {
        let Some((name, start, end, full, value)) = what else {
            self.root.set_visible(false);
            return;
        };
        self.name.set_label(name);
        self.start.set_label(start);
        self.end.set_label(end);
        self.scale.set_range(0.0, full.max(0.1));
        self.scale.set_value(value);
        self.scale
            .update_property(&[gtk::accessible::Property::Label(&format!("Open {name}"))]);
        self.root.set_visible(true);
    }
}

/// Called with a problem's index.
type ProblemCallback = Rc<dyn Fn(usize)>;

/// The problems list the status pill opens, in the left sidebar. Lane 3
/// replaces it with the problems tree and filters (Problems.dc.html); for
/// now it lists every problem, and choosing one selects it.
pub struct ProblemsList {
    /// The sidebar page.
    pub root: gtk::Box,
    /// Closes the list (back to the parts tree).
    pub close: gtk::Button,
    list: gtk::ListBox,
    on_problem: Rc<RefCell<Option<ProblemCallback>>>,
}

impl ProblemsList {
    /// An empty list.
    pub fn new() -> ProblemsList {
        let title = gtk::Label::builder()
            .label("Problems")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["title-4"])
            .build();
        let close = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .tooltip_text("Show the parts")
            .css_classes(["flat", "circular"])
            .build();
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.append(&title);
        head.append(&close);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .valign(gtk::Align::Start)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["stave-sidebar"])
            .build();
        root.append(&head);
        root.append(&scrolled);
        let on_problem: Rc<RefCell<Option<ProblemCallback>>> = Rc::new(RefCell::new(None));
        let callback = on_problem.clone();
        list.connect_row_activated(move |_, row| {
            let Ok(index) = usize::try_from(row.index()) else {
                return;
            };
            let callback = callback.borrow().clone();
            if let Some(callback) = callback {
                callback(index);
            }
        });
        ProblemsList {
            root,
            close,
            list,
            on_problem,
        }
    }

    /// Calls `callback` with a problem's index when it is chosen.
    pub fn connect_problem(&self, callback: impl Fn(usize) + 'static) {
        self.on_problem.replace(Some(Rc::new(callback)));
    }

    /// Lists the problems.
    pub fn show(&self, problems: &[Problem]) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        if problems.is_empty() {
            let row = adw::ActionRow::builder()
                .title("No problems")
                .subtitle("Everything Stave checks holds.")
                .build();
            row.set_activatable(false);
            self.list.append(&row);
            return;
        }
        for problem in problems {
            let (icon, style) = match problem.severity {
                Severity::Error => ("dialog-error-symbolic", "error"),
                Severity::Warning => ("dialog-warning-symbolic", "warning"),
                Severity::Note => ("dialog-information-symbolic", "dim-label"),
            };
            let row = adw::ActionRow::builder()
                .title(&problem.message)
                .subtitle(problem.location.as_deref().unwrap_or(""))
                .use_markup(false)
                .title_lines(3)
                .activatable(true)
                .build();
            let image = gtk::Image::from_icon_name(icon);
            image.add_css_class(style);
            row.add_prefix(&image);
            self.list.append(&row);
        }
    }
}

/// The Cut list view, until it is built.
pub fn cut_list_page() -> adw::StatusPage {
    adw::StatusPage::builder()
        .title("Cut list is coming")
        .description("Every board to cut, sized and rounded once, will be listed here. It has its own design and isn't part of this version yet. Meanwhile, “stave dump” lists every part and its size.")
        .css_classes(["stave-placeholder"])
        .build()
}
