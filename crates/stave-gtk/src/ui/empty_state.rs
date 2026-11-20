//! A docked panel's empty state ("No problems", "Nothing being added",
//! "No file open") reads whole in any frame it is docked in. Each is a
//! compact `adw::StatusPage`, drawn small by the window's sheet
//! (`style.rs`, `statuspage.compact`); in a frame too short even for that,
//! the page drops its icon, and if the words still don't fit it shows a
//! real scrollbar beside them rather than a hidden overlay one, so nothing
//! is cut off without a way to see it. The icon comes back when the frame
//! grows tall enough for it again.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

/// Makes every status page under `root` (itself included) fit its frame
/// as the module says. The workbench calls it on each panel's content
/// when it docks it; a page put into a panel later is fitted by its
/// owner with [`fit`].
pub fn fit_every(root: &gtk::Widget) {
    if let Some(page) = root.downcast_ref::<adw::StatusPage>() {
        fit(page);
        return;
    }
    let mut child = root.first_child();
    while let Some(next) = child {
        fit_every(&next);
        child = next.next_sibling();
    }
}

/// Makes one status page fit its frame: a scrollbar that shows when the
/// words overflow, and no icon while there is no room for it. A page
/// already fitted is left as it is.
pub fn fit(page: &adw::StatusPage) {
    if page.has_css_class(FITTED) {
        return;
    }
    page.add_css_class(FITTED);
    // libadwaita's page is a scrolled window around its icon and words.
    let Some(scrolled) = page
        .first_child()
        .and_then(|child| child.downcast::<gtk::ScrolledWindow>().ok())
    else {
        return;
    };
    scrolled.set_overlay_scrolling(false);
    let fitting = Rc::new(Fitting {
        page: page.downgrade(),
        scrolled: scrolled.downgrade(),
        hidden_icon: Cell::new(None),
        icon_room: Cell::new(0),
        queued: Cell::new(false),
    });
    // The adjustment changes when the frame or the words change size. The
    // icon is hidden or shown after the layout pass, not inside it.
    scrolled.vadjustment().connect_changed(move |_| {
        if fitting.queued.replace(true) {
            return;
        }
        let fitting = Rc::clone(&fitting);
        glib::idle_add_local_once(move || {
            fitting.queued.set(false);
            fitting.refit();
        });
    });
}

/// Marks a page [`fit`] has already fitted.
const FITTED: &str = "stave-fitted";

/// One page's fitting state.
struct Fitting {
    page: glib::WeakRef<adw::StatusPage>,
    scrolled: glib::WeakRef<gtk::ScrolledWindow>,
    /// The icon's name while it is hidden for want of room.
    hidden_icon: Cell<Option<glib::GString>>,
    /// The height, in pixels, that showing the icon adds.
    icon_room: Cell<i32>,
    /// Whether a refit is already waiting for the main loop.
    queued: Cell<bool>,
}

impl Fitting {
    /// Hides the icon if the page overflows its frame, or shows it again
    /// if the frame has room for it.
    fn refit(&self) {
        let (Some(page), Some(scrolled)) = (self.page.upgrade(), self.scrolled.upgrade()) else {
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
        let hidden = self.hidden_icon.take();
        match hidden {
            None => {
                if need() > room
                    && let Some(icon) = page.icon_name()
                {
                    let with_icon = need();
                    page.set_icon_name(None);
                    self.icon_room.set(with_icon.saturating_sub(need()));
                    self.hidden_icon.set(Some(icon));
                }
            }
            Some(icon) => {
                if need().saturating_add(self.icon_room.get()) <= room {
                    page.set_icon_name(Some(&icon));
                } else {
                    self.hidden_icon.set(Some(icon));
                }
            }
        }
    }
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
            fit_every(panel.root.upcast_ref());
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
