//! Saving pictures: exporting the 3D model, and the window screenshot
//! taken for the docs and tests.

use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use anyhow::{Context as _, Result};
use gtk::{gio, glib};

use super::Window;
use super::actions::close_for_good;

impl Window {
    /// Saves the view as posed as a glTF file, asking where.
    pub(super) fn export_view(self: &Rc<Self>) {
        let name = stave_app::export_view::default_file_name(
            self.session.borrow().drawable().map(|(design, _)| design),
        );
        let dialog = gtk::FileDialog::builder()
            .title("Export 3D Model (.glb, for other 3D programs)")
            .initial_name(name)
            .modal(true)
            .build();
        let window = Rc::downgrade(self);
        dialog.save(
            Some(&self.widgets.window),
            gio::Cancellable::NONE,
            move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                let Some(window) = window.upgrade() else {
                    return;
                };
                // The view as posed now, copied so that making and writing
                // the file happen off the main thread with no borrow held.
                let view = {
                    let session = window.session.borrow();
                    let state = window.state.borrow();
                    session
                        .drawable()
                        .zip(state.posed.as_ref())
                        .map(|((_, graph), posed)| (graph.clone(), posed.clone()))
                };
                let Some((graph, posed)) = view else {
                    window.show_export_failure(&stave_app::save_failure::SaveFailure {
                        heading: "Couldn't Export the View".to_owned(),
                        body: "There is nothing in the 3D view to save yet. Wait for the design to show, then try again.".to_owned(),
                    });
                    return;
                };
                let exporting = adw::Toast::builder()
                    .title("Exporting the view…")
                    .timeout(0)
                    .build();
                window.widgets.toasts.add_toast(exporting.clone());
                let window = Rc::downgrade(&window);
                glib::spawn_future_local(async move {
                    let target = path.clone();
                    let saved = gio::spawn_blocking(move || {
                        stave_app::export_view::save_glb(&graph, &posed, &target).map_err(
                            |error| stave_app::save_failure::describe_export(&error, &target),
                        )
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(stave_app::save_failure::SaveFailure {
                            heading: "Couldn't Export the View".to_owned(),
                            body: "Stave stopped part way through making the file. Nothing was saved; try again.".to_owned(),
                        })
                    });
                    exporting.dismiss();
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    match saved {
                        Ok(()) => window.toast(&format!("Saved the view as {}", path.display())),
                        Err(failure) => window.show_export_failure(&failure),
                    }
                });
            },
        );
    }

    /// Shows a failed export of the view. It stays until it is answered,
    /// and offers another place to save to.
    pub(super) fn show_export_failure(
        self: &Rc<Self>,
        failure: &stave_app::save_failure::SaveFailure,
    ) {
        let alert = adw::AlertDialog::builder()
            .heading(&failure.heading)
            .body(&failure.body)
            .close_response("cancel")
            .default_response("again")
            .build();
        alert.add_responses(&[("cancel", "_Cancel"), ("again", "Save _Elsewhere…")]);
        alert.set_response_appearance("again", adw::ResponseAppearance::Suggested);
        let again = Rc::clone(self);
        alert.connect_response(None, move |_, response| {
            if response == "again" {
                again.export_view();
            }
        });
        alert.present(Some(&self.widgets.window));
    }

    /// With `--window-screenshot`: saves the window shortly (once GTK has
    /// drawn it) and closes it.
    pub(super) fn capture_if_asked(&self) {
        let Some(path) = self.state.borrow_mut().window_screenshot.take() else {
            return;
        };
        let window = self.widgets.window.clone();
        // What `--type` typed is never saved, and closing doesn't ask.
        let files = self.widgets.files.clone();
        // An open finder would keep the window from closing.
        let finder = self.widgets.finder.dialog.clone();
        // The paintable follows the window's drawing from now on.
        let paintable = gtk::WidgetPaintable::new(Some(&window));
        let mut waited = 0;
        glib::timeout_add_local(Duration::from_millis(300), move || {
            // Wait until the window is on screen and drawn.
            waited += 1;
            // A tick makes the frame clock run, so GTK draws the window.
            window.queue_draw();
            window.add_tick_callback(|_, _| glib::ControlFlow::Break);
            if (!window.is_mapped() || waited < 3) && waited < 25 {
                return glib::ControlFlow::Continue;
            }
            // A view that hasn't drawn since it changed draws nothing yet:
            // try again on the next round.
            let Some(node) = window_node(&window, &paintable) else {
                if waited < 25 {
                    return glib::ControlFlow::Continue;
                }
                eprintln!("error: the window drew nothing");
                files.forget_unsaved();
                finder.force_close();
                close_for_good(&window);
                return glib::ControlFlow::Break;
            };
            if let Err(error) = save_node(&window, &node, &path) {
                eprintln!("error: {error:#}");
            }
            files.forget_unsaved();
            finder.force_close();
            close_for_good(&window);
            glib::ControlFlow::Break
        });
    }
}

/// The window's drawing, when it has drawn.
pub(super) fn window_node(
    window: &adw::ApplicationWindow,
    paintable: &gtk::WidgetPaintable,
) -> Option<gtk::gsk::RenderNode> {
    let snapshot = gtk::Snapshot::new();
    // In device pixels (GDK_SCALE=2 doubles the picture), so a HiDPI
    // window's sharpness can be checked.
    let scale = window
        .native()
        .and_then(|native| native.surface())
        .map_or_else(
            || f64::from(window.scale_factor()),
            |surface| surface.scale(),
        );
    let scale = crate::convert::f32_of(scale);
    snapshot.scale(scale, scale);
    paintable.snapshot(
        &snapshot,
        f64::from(window.width()),
        f64::from(window.height()),
    );
    snapshot.to_node()
}

/// Renders the window's drawing into a PNG.
pub(super) fn save_node(
    window: &adw::ApplicationWindow,
    node: &gtk::gsk::RenderNode,
    path: &Path,
) -> Result<()> {
    // A software renderer of our own: it works on every display backend.
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer
        .realize_for_display(&gtk::prelude::WidgetExt::display(window))
        .context("can't start a renderer for the window")?;
    let texture = renderer.render_texture(node, None);
    renderer.unrealize();
    texture
        .save_to_png(path)
        .with_context(|| format!("can't write '{}'", path.display()))?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

/// Screenshots look the same on every machine: GNOME's default font at
/// 96 dpi and the Adwaita icons, whatever the desktop's settings.
pub(super) fn settle_for_screenshot() {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_font_name(Some("Adwaita Sans 11"));
        settings.set_gtk_xft_dpi(96 * 1024);
        settings.set_gtk_icon_theme_name(Some("Adwaita"));
        settings.set_gtk_decoration_layout(Some(":close"));
        settings.set_gtk_enable_animations(false);
    }
}
