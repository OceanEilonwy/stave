//! A docked panel's empty state ("No problems", "Nothing being added",
//! "No file open") reads whole in any frame it is docked in. Each is a
//! compact `adw::StatusPage`, drawn small by the window's sheet
//! (`style.rs`, `statuspage.compact`); in a frame too short even for that,
//! the page drops its icon, and if the words still don't fit it shows a
//! real scrollbar beside them rather than a hidden overlay one, so nothing
//! is cut off without a way to see it. The icon comes back when the frame
//! grows tall enough for it again.
//!
//! The page's state (its hidden icon) lives in [`PageFit`], a GObject
//! subclass of `gtk::Adjustment` that the page's scrolled window uses as
//! its vertical adjustment: the scrolled window owns it, it hears every
//! change of the frame's or the words' size in its own `changed` method,
//! and the refit it waits to run after the layout pass holds it weakly.
//! No Rust state is shared between closures (docs/relm4-migration.md,
//! "Accepted exceptions").

use std::cell::Cell;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use super::*;

    /// A page's fitting state.
    #[derive(Default)]
    pub struct PageFit {
        /// The page fitted.
        pub page: glib::WeakRef<adw::StatusPage>,
        /// The icon's name while it is hidden for want of room.
        pub hidden_icon: Cell<Option<glib::GString>>,
        /// The height, in pixels, that showing the icon adds.
        pub icon_room: Cell<i32>,
        /// Whether a refit is already waiting for the main loop.
        pub queued: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PageFit {
        const NAME: &'static str = "StavePageFit";
        type Type = super::PageFit;
        type ParentType = gtk::Adjustment;
    }

    impl ObjectImpl for PageFit {}

    impl AdjustmentImpl for PageFit {
        /// The frame or the words changed size: refit once the layout
        /// pass is over (hiding the icon inside it would resize the page
        /// while it is being laid out).
        fn changed(&self) {
            self.parent_changed();
            if self.queued.replace(true) {
                return;
            }
            let fit = self.obj().downgrade();
            glib::idle_add_local_once(move || {
                if let Some(fit) = fit.upgrade() {
                    fit.imp().queued.set(false);
                    fit.refit();
                }
            });
        }
    }
}

glib::wrapper! {
    /// The vertical adjustment of a fitted status page's scrolled window,
    /// holding the page's fitting state.
    pub struct PageFit(ObjectSubclass<imp::PageFit>) @extends gtk::Adjustment;
}

impl PageFit {
    /// Hides the icon if the page overflows its frame, or shows it again
    /// if the frame has room for it.
    fn refit(&self) {
        let imp = self.imp();
        let Some(page) = imp.page.upgrade() else {
            return;
        };
        let Some(scrolled) = scrolled_window(&page) else {
            return;
        };
        let Some(content) = scrolled.child() else {
            return;
        };
        let (room, width) = (scrolled.height(), scrolled.width());
        if room <= 0 || width <= 0 {
            return;
        }
        let need = || content.measure(gtk::Orientation::Vertical, width).1;
        match imp.hidden_icon.take() {
            None => {
                if need() > room
                    && let Some(icon) = page.icon_name()
                {
                    let with_icon = need();
                    page.set_icon_name(None);
                    imp.icon_room.set(with_icon.saturating_sub(need()));
                    imp.hidden_icon.set(Some(icon));
                }
            }
            Some(icon) => {
                if need().saturating_add(imp.icon_room.get()) <= room {
                    page.set_icon_name(Some(&icon));
                } else {
                    imp.hidden_icon.set(Some(icon));
                }
            }
        }
    }
}

/// libadwaita's page is a scrolled window around its icon and words.
fn scrolled_window(page: &adw::StatusPage) -> Option<gtk::ScrolledWindow> {
    page.first_child()
        .and_then(|child| child.downcast::<gtk::ScrolledWindow>().ok())
}

/// Makes one status page fit its frame: a scrollbar that shows when the
/// words overflow, and no icon while there is no room for it. A page
/// already fitted is left as it is. Each component that docks a status
/// page in a panel calls it on its own page (the Problems, Files, Code,
/// Details and Edit panels); nothing reaches into another's page.
pub fn fit(page: &adw::StatusPage) {
    let Some(scrolled) = scrolled_window(page) else {
        return;
    };
    if scrolled.vadjustment().is::<PageFit>() {
        return;
    }
    scrolled.set_overlay_scrolling(false);
    let fit: PageFit = glib::Object::new();
    fit.imp().page.set(Some(page));
    scrolled.set_vadjustment(Some(&fit));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::edit_panel::EditPanel;
    use crate::ui::gtk_test::{on_gtk_thread, settle};

    /// Lays the window out at 400x400 now (a window off screen may get
    /// no frames to lay itself out in), lets the page refit, and lays it
    /// out again to show the refit.
    fn lay_out(window: &gtk::Window) {
        for _ in 0..3 {
            window.allocate(400, 400, -1, None);
            settle();
        }
    }

    /// The page's scrolled window.
    fn scrolled(page: &adw::StatusPage) -> gtk::ScrolledWindow {
        page.first_child()
            .and_then(|child| child.downcast::<gtk::ScrolledWindow>().ok())
            .expect("the page's scrolled window")
    }

    /// Every status page under `root`.
    fn pages_under(root: &gtk::Widget) -> Vec<adw::StatusPage> {
        let mut found = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(widget) = stack.pop() {
            if let Ok(page) = widget.clone().downcast::<adw::StatusPage>() {
                found.push(page);
                continue;
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                stack.push(current);
            }
        }
        found
    }

    /// Each panel with an empty state fits its own page when it is made:
    /// nothing outside it (the workbench docking it) reaches in.
    #[test]
    fn every_docked_empty_state_is_fitted_by_its_own_panel() {
        on_gtk_thread(|| {
            let (problems_to, _) = crate::ui::gtk_test::recorder();
            let problems = crate::ui::panels::ProblemsPanel::new(&problems_to, |output| output);
            let (files_to, _) = crate::ui::gtk_test::recorder();
            let files = crate::ui::panels::FilesList::new(&files_to, |output| output);
            let (inspector_to, _) = crate::ui::gtk_test::recorder::<()>();
            let inspector = crate::ui::inspector::Inspector::new(&inspector_to, |_| (), |_| ());
            let (code_to, _) = crate::ui::gtk_test::recorder();
            let code = crate::ui::files::FilesPage::<()>::new(&code_to, None, |_| ());
            let edit = EditPanel::new(&[]);
            for root in [
                problems.root.clone().upcast::<gtk::Widget>(),
                files.root.clone().upcast(),
                inspector.root.clone().upcast(),
                code.root.clone().upcast(),
                edit.root.clone().upcast(),
            ] {
                let pages = pages_under(&root);
                assert!(
                    !pages.is_empty(),
                    "{} has an empty state",
                    root.type_().name()
                );
                for page in pages {
                    assert!(
                        scrolled(&page).vadjustment().is::<PageFit>(),
                        "{}'s empty state is fitted",
                        root.type_().name()
                    );
                }
            }
            settle();
        });
    }

    /// The Edit panel's empty state, fitted, in a 400x400 window, with a
    /// spacer above it and one beside it to make its frame smaller.
    struct Shown {
        page: adw::StatusPage,
        above: gtk::Box,
        beside: gtk::Box,
        window: gtk::Window,
    }

    impl Shown {
        fn new() -> Shown {
            let panel = EditPanel::new(&[]);
            let page = panel
                .root
                .child_by_name("empty")
                .and_then(|page| page.downcast::<adw::StatusPage>().ok())
                .expect("the Edit panel's empty state");
            let beside = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            row.set_vexpand(true);
            row.append(&beside);
            row.append(&panel.root);
            let above = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.append(&above);
            column.append(&row);
            let window = gtk::Window::builder()
                .default_width(400)
                .default_height(400)
                .child(&column)
                .build();
            // The Edit panel fits its own page (`EditPanel::new`).
            window.present();
            lay_out(&window);
            Shown {
                page,
                above,
                beside,
                window,
            }
        }

        /// Makes the frame `width` by `height`.
        fn frame(&self, width: i32, height: i32) {
            self.above.set_size_request(-1, 400 - height);
            self.beside.set_size_request(400 - width, -1);
            lay_out(&self.window);
        }
    }

    /// Whether every word of the page is in sight, or a scrollbar shows
    /// that there is more.
    fn readable(page: &adw::StatusPage) -> bool {
        let scrolled = scrolled(page);
        let adjustment = scrolled.vadjustment();
        adjustment.upper() <= adjustment.page_size() + 0.5
            || scrolled.vscrollbar().is_child_visible()
    }

    /// The Edit panel's empty state (the longest) reads whole, or shows a
    /// scrollbar, in every frame it can be docked in: narrow and short,
    /// at the bottom dock's height (190), and tall. In a short frame it
    /// drops its icon, and a frame too short even for its words shows a
    /// scrollbar; in a tall frame the icon comes back. Without the
    /// window's own sheet the page is libadwaita's larger compact one, so
    /// this is the harder case.
    #[test]
    fn the_edit_empty_state_reads_whole_in_a_short_frame() {
        on_gtk_thread(|| {
            let shown = Shown::new();
            let page = &shown.page;
            let icon = page.icon_name();
            assert!(icon.is_some());
            assert!(!scrolled(page).is_overlay_scrolling());

            shown.frame(200, 120);
            assert_eq!(page.icon_name(), None, "no room for the icon");
            assert!(
                scrolled(page).vscrollbar().is_child_visible(),
                "the words overflow, and the scrollbar says so"
            );

            for (width, height) in [(200, 190), (300, 190), (400, 190), (240, 400), (200, 60)] {
                shown.frame(width, height);
                assert!(readable(page), "{width}x{height}");
            }

            shown.frame(400, 400);
            assert_eq!(page.icon_name(), icon, "room for the icon again");
            assert!(!scrolled(page).vscrollbar().is_child_visible());
            shown.window.destroy();
        });
    }
}
