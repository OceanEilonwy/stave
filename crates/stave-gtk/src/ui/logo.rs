//! Stave's logo, the dovetail cube (the application icon), everywhere the
//! window shows it.
//!
//! - [`install_icons`] adds Stave's own icons, built into the binary as a
//!   GResource (`build.rs`), to the icon theme, so the application icon
//!   resolves by name whether or not Stave is installed: the window icon
//!   that the title bar draws when the desktop's button layout asks for
//!   one (KDE's starts with `icon`), the About dialog's and the start
//!   page's. Without it they all drew the theme's "missing image".
//! - [`Logo::for_title_bar`] is the logo at the start of a window's title
//!   bar: drawn from its own paths in its own colours, and shown only when
//!   the button layout doesn't already put the window icon there, so a
//!   window shows the cube at its top left exactly once.
//!
//! The faces are those of
//! `data/icons/hicolor/scalable/apps/io.github.OceanEilonwy.Stave.svg`
//! (its 120-unit view box), without the ground shadow: change both
//! together.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib};

/// Where the built-in icons live in the resource bundle: a hicolor-style
/// tree (`scalable/apps/…`).
const ICONS_RESOURCE_PATH: &str = "/io/github/OceanEilonwy/Stave/icons";

/// Registers the icons built into the binary and adds them to `display`'s
/// icon theme. Safe to call more than once.
pub fn install_icons(display: &gdk::Display) {
    static REGISTERED: std::sync::Once = std::sync::Once::new();
    REGISTERED.call_once(|| {
        let bundle =
            glib::Bytes::from_static(include_bytes!(concat!(env!("OUT_DIR"), "/icons.gresource")));
        // The bundle is built by build.rs from files in this repository:
        // it can only fail to load if the build is broken, and then the
        // window shows the theme's icons as before.
        if let Ok(resource) = gio::Resource::from_data(&bundle) {
            gio::resources_register(&resource);
        }
    });
    let theme = gtk::IconTheme::for_display(display);
    if !theme
        .resource_path()
        .iter()
        .any(|path| path.as_str() == ICONS_RESOURCE_PATH)
    {
        theme.add_resource_path(ICONS_RESOURCE_PATH);
    }
}

/// `true` when a title-bar button `layout` (GTK's `gtk-decoration-layout`:
/// start items, a colon, end items, each comma-separated) puts the window
/// icon at the start.
pub fn layout_starts_with_icon(layout: &str) -> bool {
    layout
        .split(':')
        .next()
        .is_some_and(|start| start.split(',').any(|item| item.trim() == "icon"))
}

/// The logo's view box side.
const VIEW_BOX: f32 = 120.0;

/// The cube's faces, back to front: colour (as `#rrggbb`) and SVG path.
pub const FACES: [(&str, &str); 5] = [
    (
        "#7a5134",
        "M44.80 81.12 L51.56 64.22 L23.41 47.98 L30.17 72.67 L14.97 63.90 L14.97 86.00 L60.00 112.00 L60.00 89.90 Z",
    ),
    (
        "#573722",
        "M60.00 112.00 L105.03 86.00 L105.03 56.10 L89.83 64.88 L96.59 81.78 L68.44 98.03 L75.20 73.33 L60.00 82.10 Z",
    ),
    (
        "#edb866",
        "M14.97 63.90 L30.17 72.67 L23.41 47.98 L51.56 64.22 L44.80 81.12 L60.00 89.90 L60.00 60.00 L14.97 34.00 Z",
    ),
    (
        "#cf9447",
        "M60.00 60.00 L60.00 82.10 L75.20 73.33 L68.44 98.03 L96.59 81.78 L89.83 64.88 L105.03 56.10 L105.03 34.00 Z",
    ),
    (
        "#f6d79a",
        "M14.97 34.00 L60.00 60.00 L105.03 34.00 L60.00 8.00 Z",
    ),
];

mod imp {
    use super::*;

    pub struct Logo {
        pub size: Cell<i32>,
    }

    impl Default for Logo {
        fn default() -> Self {
            Logo {
                size: Cell::new(20),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Logo {
        const NAME: &'static str = "StaveLogo";
        type Type = super::Logo;
        type ParentType = gtk::Widget;

        fn class_init(class: &mut Self::Class) {
            class.set_css_name("stavelogo");
            // Decoration: the window's title names the project.
            class.set_accessible_role(gtk::AccessibleRole::Presentation);
        }
    }

    impl ObjectImpl for Logo {}

    impl WidgetImpl for Logo {
        fn measure(&self, _orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let size = self.size.get();
            (size, size, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let size = crate::convert::f32_of_int(self.size.get());
            let left = (crate::convert::f32_of_int(widget.width()) - size) / 2.0;
            let top = (crate::convert::f32_of_int(widget.height()) - size) / 2.0;
            snapshot.save();
            snapshot.translate(&gtk::graphene::Point::new(left, top));
            snapshot.scale(size / VIEW_BOX, size / VIEW_BOX);
            for (colour, data) in FACES {
                let (Ok(colour), Ok(path)) =
                    (gtk::gdk::RGBA::parse(colour), gtk::gsk::Path::parse(data))
                else {
                    continue;
                };
                snapshot.append_fill(&path, gtk::gsk::FillRule::Winding, &colour);
            }
            snapshot.restore();
        }
    }
}

glib::wrapper! {
    /// Stave's logo, the dovetail cube, drawn in its own colours.
    pub struct Logo(ObjectSubclass<imp::Logo>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Logo {
    /// The logo, `size` logical pixels square.
    pub fn new(size: i32) -> Logo {
        let logo: Logo = glib::Object::builder()
            .property("valign", gtk::Align::Center)
            .property("halign", gtk::Align::Center)
            .build();
        logo.imp().size.set(size);
        logo
    }

    /// Its side, in logical pixels.
    pub fn size(&self) -> i32 {
        self.imp().size.get()
    }

    /// The logo for the start of a window's title bar, 20 px. It hides
    /// while the desktop's button layout puts the window icon (the same
    /// cube, from [`install_icons`]) at the start, and follows the layout
    /// when it changes.
    pub fn for_title_bar() -> Logo {
        let logo = Logo::new(20);
        logo.set_margin_start(4);
        logo.set_margin_end(2);
        let Some(settings) = gtk::Settings::default() else {
            return logo;
        };
        logo.follow_layout(&settings);
        let handler = settings.connect_gtk_decoration_layout_notify(glib::clone!(
            #[weak]
            logo,
            move |settings| logo.follow_layout(settings)
        ));
        // The settings outlive every window: let go of the handler with
        // the logo.
        let handler = Rc::new(RefCell::new(Some(handler)));
        logo.connect_destroy(move |_| {
            if let Some(handler) = handler.borrow_mut().take() {
                settings.disconnect(handler);
            }
        });
        logo
    }

    /// Shows the logo unless `settings`' button layout already draws the
    /// window icon at the start of the bar.
    fn follow_layout(&self, settings: &gtk::Settings) {
        let icon_at_start = settings
            .gtk_decoration_layout()
            .is_some_and(|layout| layout_starts_with_icon(&layout));
        self.set_visible(!icon_at_start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = include_str!(
        "../../../../data/icons/hicolor/scalable/apps/io.github.OceanEilonwy.Stave.svg"
    );

    #[test]
    fn the_logo_draws_the_application_icons_faces() {
        for (colour, data) in FACES {
            assert!(
                SVG.contains(&format!("fill=\"{colour}\" d=\"{data}\"")),
                "{colour} face differs from the icon"
            );
        }
    }

    #[test]
    fn the_window_icon_is_at_the_start_only_when_the_layout_says_so() {
        // KDE (Breeze), GNOME, macOS-style, none.
        assert!(layout_starts_with_icon("icon:minimize,maximize,close"));
        assert!(layout_starts_with_icon("close, icon:menu"));
        assert!(!layout_starts_with_icon("appmenu:minimize,maximize,close"));
        assert!(!layout_starts_with_icon(":close"));
        assert!(!layout_starts_with_icon("menu:icon,close"));
        assert!(!layout_starts_with_icon(""));
    }

    #[test]
    fn the_application_icon_resolves_without_an_install() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            let display = gdk::Display::default().expect("a display");
            install_icons(&display);
            install_icons(&display);
            let theme = gtk::IconTheme::for_display(&display);
            let id = crate::ui::app::APP_ID;
            for name in [id.to_owned(), format!("{id}-symbolic")] {
                assert!(theme.has_icon(&name), "{name} is not in the icon theme");
                let icon = theme.lookup_icon(
                    &name,
                    &[],
                    32,
                    1,
                    gtk::TextDirection::Ltr,
                    gtk::IconLookupFlags::empty(),
                );
                assert_eq!(
                    icon.icon_name().as_deref(),
                    Some(std::path::Path::new(&name)),
                    "{name} resolves to itself, not to the missing image"
                );
            }
            assert_eq!(
                theme
                    .resource_path()
                    .iter()
                    .filter(|path| path.as_str() == ICONS_RESOURCE_PATH)
                    .count(),
                1,
                "added once"
            );
        });
    }

    #[test]
    fn the_title_bar_logo_follows_the_button_layout() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            let settings = gtk::Settings::default().expect("settings");
            let before = settings.gtk_decoration_layout();
            let logo = Logo::for_title_bar();
            settings.set_gtk_decoration_layout(Some("icon:minimize,maximize,close"));
            assert!(!logo.is_visible(), "the window icon is the logo");
            settings.set_gtk_decoration_layout(Some("appmenu:minimize,maximize,close"));
            assert!(logo.is_visible(), "no window icon: the logo shows");
            settings.set_gtk_decoration_layout(before.as_deref());
        });
    }

    #[test]
    fn every_face_parses() {
        crate::ui::gtk_test::on_gtk_thread(|| {
            for (colour, data) in FACES {
                assert!(gtk::gdk::RGBA::parse(colour).is_ok(), "{colour}");
                assert!(gtk::gsk::Path::parse(data).is_ok(), "{data}");
            }
        });
    }
}
