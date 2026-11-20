//! The functions the page calls, for `wasm32` only: a renderer on the
//! page's canvas, through WebGPU, and the [`View`] it draws.
//!
//! Every call that changes what is seen draws a new frame at once; there
//! is no animation loop. Without WebGPU, [`start`] fails with
//! [`NEEDS_WEBGPU`] and nothing else runs.

use std::cell::RefCell;

use stave_render::{Backdrop, Frame, Highlighted, Renderer};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::files::Files;
use crate::page::NEEDS_WEBGPU;
use crate::project::Project;
use crate::view::View;

/// The viewer of this page.
struct Viewer {
    renderer: Renderer,
    width: u32,
    height: u32,
    view: Option<View>,
}

thread_local! {
    static VIEWER: RefCell<Option<Viewer>> = const { RefCell::new(None) };
}

/// Runs `act` on the started viewer; an error when [`start`] hasn't
/// finished.
fn with_viewer<T>(act: impl FnOnce(&mut Viewer) -> Result<T, JsError>) -> Result<T, JsError> {
    VIEWER.with(|cell| {
        let mut slot = cell
            .try_borrow_mut()
            .map_err(|_| JsError::new("the viewer is busy"))?;
        let viewer = slot
            .as_mut()
            .ok_or_else(|| JsError::new("the viewer hasn't started"))?;
        act(viewer)
    })
}

/// `true` when the browser has WebGPU (`navigator.gpu`).
fn has_webgpu() -> bool {
    web_sys::window().is_some_and(|window| {
        js_sys::Reflect::get(&window.navigator(), &JsValue::from_str("gpu"))
            .is_ok_and(|gpu| !gpu.is_undefined() && !gpu.is_null())
    })
}

impl Viewer {
    /// Draws the view on the canvas; nothing when no project is loaded or
    /// nothing placed.
    fn draw(&mut self) -> Result<(), JsError> {
        let Some(view) = &self.view else {
            return Ok(());
        };
        let (Some(graph), Some(drawing)) = (view.graph(), view.drawing()) else {
            return Ok(());
        };
        let frame = Frame {
            scene: graph,
            posed: &drawing.posed,
            orbit: drawing.orbit,
            selection: &drawing.selection,
            highlight: Some(Highlighted {
                nodes: &drawing.problem_nodes,
                volumes: &drawing.problem_volumes,
            }),
            section: None,
            looks: Some(&drawing.looks),
            overlay: None,
            tint_cut_faces: true,
            lit: None,
            tints: Some(&drawing.tints),
            backdrop: Backdrop::default(),
            width: self.width,
            height: self.height,
        };
        self.renderer
            .present(&frame)
            .map_err(|error| JsError::new(&error.to_string()))
    }
}

/// Opens WebGPU on `canvas` and gets ready to draw. Fails with "This
/// preview needs WebGPU." in a browser without it.
#[wasm_bindgen]
pub async fn start(canvas: HtmlCanvasElement) -> Result<(), JsError> {
    if !has_webgpu() {
        return Err(JsError::new(NEEDS_WEBGPU));
    }
    let (width, height) = (canvas.width().max(1), canvas.height().max(1));
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(descriptor);
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas))
        .map_err(|_| JsError::new(NEEDS_WEBGPU))?;
    // The page says only that WebGPU is needed; why it couldn't be had
    // goes to the console, for whoever looks.
    let renderer = Renderer::for_surface(instance, surface, width, height)
        .await
        .map_err(|error| {
            web_sys::console::warn_1(&JsValue::from_str(&error.to_string()));
            JsError::new(NEEDS_WEBGPU)
        })?;
    VIEWER.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = Some(Viewer {
                renderer,
                width,
                height,
                view: None,
            });
        }
    });
    Ok(())
}

/// Shows the project in `folder` whose files `files_json` holds (a JSON
/// object of paths to texts), with the suggestion whose changed files
/// `suggestion_json` holds over it (the same form; empty or `null` for
/// none). Returns the summary the page shows beside the view, as JSON
/// ([`View::summary_json`]).
#[wasm_bindgen]
pub fn load(folder: &str, files_json: &str, suggestion_json: &str) -> Result<String, JsError> {
    let files = Files::from_json(folder, files_json)?;
    let suggestion = match suggestion_json.trim() {
        "" | "null" => None,
        json => Some(Files::from_json(folder, json)?),
    };
    let project = Project::open(&files, suggestion.as_ref())?;
    with_viewer(|viewer| {
        let view = View::new(project, viewer.width, viewer.height);
        if let Some(graph) = view.graph() {
            viewer.renderer.upload(graph);
        }
        let summary = view.summary_json();
        viewer.view = Some(view);
        viewer.draw()?;
        Ok(summary)
    })
}

/// Opens motion `motion_index` (in the summary's `motions` order) to
/// `amount`, from 0 (closed) to 1 (fully open).
#[wasm_bindgen]
pub fn set_pose(motion_index: usize, amount: f32) -> Result<(), JsError> {
    with_viewer(|viewer| {
        if let Some(view) = &mut viewer.view {
            view.set_pose(motion_index, amount)?;
        }
        viewer.draw()
    })
}

/// Pulls the design apart by `amount`, from 0 (as built) to 1 (fully
/// exploded).
#[wasm_bindgen]
pub fn set_explode(amount: f32) -> Result<(), JsError> {
    with_viewer(|viewer| {
        if let Some(view) = &mut viewer.view {
            view.set_explode(amount);
        }
        viewer.draw()
    })
}

/// Turns the design by a drag of `(dx, dy)` CSS pixels.
#[wasm_bindgen]
pub fn orbit(dx: f32, dy: f32) -> Result<(), JsError> {
    with_viewer(|viewer| {
        if let Some(view) = &mut viewer.view {
            view.orbit_by(dx, dy);
        }
        viewer.draw()
    })
}

/// Closer (`steps > 0`) or further away.
#[wasm_bindgen]
pub fn zoom(steps: f32) -> Result<(), JsError> {
    with_viewer(|viewer| {
        if let Some(view) = &mut viewer.view {
            view.zoom(steps);
        }
        viewer.draw()
    })
}

/// Follows the canvas's new size in device pixels.
#[wasm_bindgen]
pub fn resize(width: u32, height: u32) -> Result<(), JsError> {
    with_viewer(|viewer| {
        viewer.width = width.max(1);
        viewer.height = height.max(1);
        viewer.renderer.resize(viewer.width, viewer.height);
        if let Some(view) = &mut viewer.view {
            view.resize(viewer.width, viewer.height);
        }
        viewer.draw()
    })
}

/// What is under the device pixel `(x, y)` of the canvas, as JSON
/// ([`crate::Picked::to_json`]), selecting it; `null` when nothing is.
#[wasm_bindgen]
pub fn pick(x: f32, y: f32) -> Result<String, JsError> {
    with_viewer(|viewer| {
        let picked = viewer.view.as_mut().and_then(|view| view.pick(x, y));
        viewer.draw()?;
        Ok(picked.map_or_else(|| "null".to_owned(), |picked| picked.to_json()))
    })
}
