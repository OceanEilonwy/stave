//! The smaller pieces of the window: the status bar, the floating motion
//! bar at the bottom of the 3D view ("Open it"), and the placeholder pages
//! of the Cut list and Files views.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use stave_app::session::{CheckStatus, SourceLocation, Tone};

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

/// The Cut list view, until it is built.
pub fn cut_list_page() -> adw::StatusPage {
    adw::StatusPage::builder()
        .title("Cut list is coming")
        .description("Every board to cut, sized and rounded once, will be listed here. It has its own design and isn't part of this version yet. Meanwhile, “stave dump” lists every part and its size.")
        .css_classes(["stave-placeholder"])
        .build()
}

/// The Files view, until lane 2's editor lands: says which place a link
/// asked for.
pub struct FilesPage {
    /// The page.
    pub root: adw::StatusPage,
}

impl FilesPage {
    /// The placeholder.
    pub fn new() -> FilesPage {
        let root = adw::StatusPage::builder()
            .title("Files")
            .description("The project's files and the editor will be here.")
            .css_classes(["stave-placeholder"])
            .build();
        FilesPage { root }
    }

    /// Shows which place is selected.
    pub fn show(&self, location: Option<&SourceLocation>, folder: &std::path::Path) {
        let description = match location {
            Some(location) => format!(
                "The editor will open {} here. It isn't built yet.",
                location.display(folder)
            ),
            None => {
                "The project's files and the editor will be here. They aren't built yet.".to_owned()
            }
        };
        self.root.set_description(Some(&description));
    }
}
