//! The preview: `preview`, `show_in_stave` and `render_view`, and the
//! MCP App page `preview` points at.
//!
//! ```text
//! preview ──{folder, files, suggestion}──┬──▶ MCP App page ui://stave/viewer (Claude Desktop)
//!                                        └──▶ ViewerServer on 127.0.0.1 (a link, every host)
//! show_in_stave ──D-Bus──▶ the Stave window (the suggestion as a ghost)
//! render_view ──stave-render, offscreen──▶ a PNG
//! ```
//!
//! The page is the viewer `tools/build-viewer.sh` builds (`stave-web`),
//! embedded when the server is built, with a small script for its host
//! added: inside an AI app it speaks the MCP Apps bridge (`ui/initialize`,
//! then the tool result arrives as `ui/notifications/tool-result`) and in
//! a browser it polls the localhost viewer's `/scene`. Either way it hands
//! the project to `staveViewer.show` and calls no tool: the preview never
//! changes anything. Every tool's text stands on its own, since Claude
//! Code and Codex show no page.
//!
//! The viewer is WebAssembly, and the MCP Apps spec's default Content
//! Security Policy for a page (`script-src 'self' 'unsafe-inline'`) has
//! no `'wasm-unsafe-eval'`, nor any `_meta.ui.csp` field to ask for it. A
//! host that applies that policy as it stands keeps the page from
//! compiling the viewer; the page then says so and points to the link in
//! `preview`'s answer, which opens the same preview in a web browser.

use std::collections::BTreeMap;
use std::path::Path;

use base64::Engine as _;
use rmcp::ErrorData;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, MetaObject, ReadResourceResult, Resource, ResourceContents,
};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, json};
use stave_history::{ProjectFiles, RevisionId, SuggestionId};
use stave_link::{LinkError, Shown};
use stave_render::{Backdrop, Frame, RenderError, Renderer};
use stave_scene::Orbit;

use super::projects::open;
use super::{blocking, finish};
use crate::agent_line::{self, Replay};
use crate::tools::changing::with_header;
use crate::{McpError, StaveServer};

/// The MCP App page's resource.
pub const VIEWER_URI: &str = "ui://stave/viewer";

/// The MIME type of an MCP App page.
pub const APP_MIME_TYPE: &str = "text/html;profile=mcp-app";

/// The viewer page as built, or a page saying how to build it.
const BUILT_PAGE: &str = include_str!(concat!(env!("OUT_DIR"), "/stave-viewer.html"));

/// The MCP Apps protocol version the page speaks.
const APPS_PROTOCOL_VERSION: &str = "2026-01-26";

/// The page's height in an AI app, in CSS pixels.
const APP_HEIGHT: u32 = 560;

/// `render_view`'s picture size, in pixels.
const PICTURE_WIDTH: u32 = 800;
/// See [`PICTURE_WIDTH`].
const PICTURE_HEIGHT: u32 = 600;

/// The script that makes the page an MCP App: it asks its host to begin
/// (`ui/initialize`), says it is ready, asks for its height, and shows
/// every tool result that carries a project. It answers the host's
/// teardown and refuses every other request; it sends no request but
/// `ui/initialize`, so it never calls a tool.
const APP_BRIDGE: &str = r#"(function () {
  "use strict";
  if (window.parent === window) return;
  var nextId = 1;
  var waiting = {};
  function post(message) { window.parent.postMessage(message, "*"); }
  function request(method, params) {
    var id = nextId++;
    post({ jsonrpc: "2.0", id: id, method: method, params: params });
    return new Promise(function (resolve) { waiting[id] = resolve; });
  }
  function notify(method, params) {
    post({ jsonrpc: "2.0", method: method, params: params || {} });
  }
  function show(result) {
    var project = result && result.structuredContent;
    if (project && project.files && window.staveViewer) window.staveViewer.show(project);
  }
  window.addEventListener("message", function (event) {
    if (event.source !== window.parent) return;
    var message = event.data;
    if (!message || message.jsonrpc !== "2.0") return;
    if (message.method === undefined) {
      var resolve = waiting[message.id];
      if (resolve) { delete waiting[message.id]; resolve(message.result); }
      return;
    }
    if (message.method === "ui/notifications/tool-result") {
      show(message.params);
    } else if (message.id !== undefined) {
      if (message.method === "ui/resource-teardown") {
        post({ jsonrpc: "2.0", id: message.id, result: {} });
      } else {
        post({ jsonrpc: "2.0", id: message.id,
               error: { code: -32601, message: "Method not found" } });
      }
    }
  });
  request("ui/initialize", {
    protocolVersion: "@PROTOCOL@",
    appInfo: { name: "Stave preview", version: "@VERSION@" },
    appCapabilities: {}
  }).then(function () {
    notify("ui/notifications/initialized");
    notify("ui/notifications/size-changed",
           { width: document.documentElement.clientWidth, height: @HEIGHT@ });
  });
})();"#;

/// The script that makes the page the localhost viewer's: it reads the
/// token from its own link and polls `/scene` on the same origin, showing
/// each new project the AI app previews.
const LOCAL_BRIDGE: &str = r#"(function () {
  "use strict";
  var token = new URLSearchParams(window.location.search).get("token") || "";
  var shown = -1;
  function poll() {
    fetch("/scene?token=" + encodeURIComponent(token), { cache: "no-store" })
      .then(function (response) { return response.ok ? response.json() : null; })
      .then(function (scene) {
        if (scene && scene.project && scene.version !== shown && window.staveViewer) {
          shown = scene.version;
          window.staveViewer.show(scene.project);
        }
      })
      .catch(function () {})
      .then(function () { window.setTimeout(poll, 2000); });
  }
  poll();
})();"#;

/// `script` put into `page` just before its `</body>`, or at its end.
fn with_script(page: &str, script: &str) -> String {
    let tag = format!("<script>{script}</script>\n");
    match page.rfind("</body>") {
        Some(end) => {
            let mut whole = String::with_capacity(page.len().saturating_add(tag.len()));
            whole.push_str(page.get(..end).unwrap_or(page));
            whole.push_str(&tag);
            whole.push_str(page.get(end..).unwrap_or_default());
            whole
        }
        None => format!("{page}{tag}"),
    }
}

/// The MCP App page: the viewer with the Apps bridge.
pub fn app_page() -> String {
    let bridge = APP_BRIDGE
        .replace("@PROTOCOL@", APPS_PROTOCOL_VERSION)
        .replace("@VERSION@", env!("CARGO_PKG_VERSION"))
        .replace("@HEIGHT@", &APP_HEIGHT.to_string());
    with_script(BUILT_PAGE, &bridge)
}

/// The localhost viewer's page: the viewer polling its server.
pub fn local_page() -> String {
    with_script(BUILT_PAGE, LOCAL_BRIDGE)
}

/// The `_meta` that ties `preview` to its page: `ui.resourceUri`, as the
/// MCP Apps spec has it, and the older flat `ui/resourceUri` for hosts
/// that read only that.
fn ui_meta() -> MetaObject {
    let mut meta = Map::new();
    meta.insert("ui".to_owned(), json!({ "resourceUri": VIEWER_URI }));
    meta.insert("ui/resourceUri".to_owned(), json!(VIEWER_URI));
    MetaObject(meta)
}

/// The page's `_meta` on the resource: a border, and no outside origin
/// (none is declared, so the host's strictest rules hold).
fn page_meta() -> MetaObject {
    let mut meta = Map::new();
    meta.insert("ui".to_owned(), json!({ "prefersBorder": true }));
    MetaObject(meta)
}

/// The MCP App page, as `resources/list` lists it.
pub fn viewer_resource() -> Resource {
    Resource::new(VIEWER_URI, "stave-viewer")
        .with_title("Stave preview")
        .with_description(
            "The 3D preview of a Stave project: turn it, zoom, click a part, open doors and \
             drawers, pull it apart, and see a suggestion as a ghost. It never changes the project.",
        )
        .with_mime_type(APP_MIME_TYPE)
        .with_meta(page_meta())
}

/// `resources/read` for `uri`.
///
/// # Errors
///
/// When `uri` isn't the viewer's.
pub fn read_viewer(uri: &str) -> Result<ReadResourceResult, ErrorData> {
    if uri != VIEWER_URI {
        return Err(ErrorData::resource_not_found(
            format!("There is no resource '{uri}'. The only one is {VIEWER_URI}."),
            None,
        ));
    }
    Ok(ReadResourceResult::new(vec![
        ResourceContents::text(app_page(), VIEWER_URI)
            .with_mime_type(APP_MIME_TYPE)
            .with_meta(page_meta()),
    ]))
}

/// The arguments of `preview` and `show_in_stave`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreviewArguments {
    /// A suggestion's id, from suggest or list_suggestions, to show as a
    /// ghost over the project; without it the project is shown as it is.
    #[serde(default)]
    pub suggestion: Option<String>,
}

/// `render_view`'s arguments.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct RenderArguments {
    /// Where the camera is; anything left out frames the whole design
    /// from the usual view (from the front, to the right and above).
    #[serde(default)]
    pub camera: Option<Camera>,
}

/// A camera orbiting the design's centre.
#[derive(Debug, Default, Clone, Copy, Deserialize, JsonSchema)]
pub struct Camera {
    /// Degrees around the design, from the front (0) toward the right
    /// side (90).
    #[serde(default)]
    pub yaw: Option<f32>,
    /// Degrees above the horizontal, from -89 (from below) to 89 (from
    /// above).
    #[serde(default)]
    pub pitch: Option<f32>,
    /// Millimetres from the design's centre.
    #[serde(default)]
    pub distance: Option<f32>,
}

impl Camera {
    /// `orbit` moved where this camera says; the values are checked and
    /// turned into the camera's own at the rendering boundary
    /// ([`Orbit::placed`]).
    fn place(self, orbit: Orbit) -> Result<Orbit, McpError> {
        orbit
            .placed(self.yaw, self.pitch, self.distance)
            .map_err(|error| McpError::Camera(error.to_string()))
    }
}

#[tool_router(router = preview_router, vis = "pub(crate)")]
impl StaveServer {
    /// The 3D preview.
    #[tool(
        description = "Show the open project in 3D, and a suggestion as a ghost over it when \
                       given its id. In Claude Desktop the preview appears in the conversation; \
                       every app also gets a link to the same preview for a web browser. The \
                       person can turn it, zoom, click a part to see what it is, open doors and \
                       drawers and pull the design apart. It changes nothing.",
        annotations(read_only_hint = true, open_world_hint = false),
        meta = ui_meta()
    )]
    async fn preview(&self, Parameters(arguments): Parameters<PreviewArguments>) -> CallToolResult {
        finish(self.show_preview(arguments.suggestion).await)
    }

    /// Shows the project in the Stave window.
    #[tool(
        description = "Ask the Stave window, when it is running, to open the project and show \
                       a suggestion (given its id) as a ghost over it. Nothing is written: the \
                       person decides in Stave.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn show_in_stave(
        &self,
        Parameters(arguments): Parameters<PreviewArguments>,
    ) -> CallToolResult {
        finish(self.show_in_window(arguments.suggestion).await)
    }

    /// A picture of the design.
    #[tool(
        description = "Draw a picture (PNG) of the open project, for an app that shows images \
                       but not the 3D preview. camera sets yaw and pitch in degrees and \
                       distance in millimetres; without it the whole design is framed.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn render_view(
        &self,
        Parameters(arguments): Parameters<RenderArguments>,
    ) -> CallToolResult {
        finish(self.picture(arguments.camera.unwrap_or_default()).await)
    }
}

impl StaveServer {
    async fn show_preview(&self, suggestion: Option<String>) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let folder = project.folder.clone();
        let (shown, title) = blocking(move || {
            let mut files = project_files(&folder)?;
            let mut title = None;
            let mut fields = Map::new();
            if let Some(id) = suggestion {
                let view = suggestion_view(&folder, &id)?;
                fields.insert("suggestion".to_owned(), json!(view.changes));
                fields.insert("suggestion_id".to_owned(), json!(id));
                fields.insert("suggestion_title".to_owned(), json!(view.title));
                match &view.over {
                    Over::Project => {
                        fields.insert("suggestion_over".to_owned(), json!("project"));
                    }
                    Over::Base {
                        revision,
                        files: base,
                    } => {
                        // The ghost is drawn over the project as the
                        // suggestion found it, so the person's later
                        // edits aren't shown as the suggestion undoing them.
                        // Only the files outside the folder (libraries
                        // beside it, keyed by full path) stay from the
                        // folder as it is now; everything inside it is
                        // the base's, so a file added since isn't shown.
                        files.retain(|key, _| Path::new(key).is_absolute());
                        files.extend(text_files(base));
                        fields.insert("suggestion_over".to_owned(), json!("base"));
                        fields.insert("suggestion_base".to_owned(), json!(revision.as_str()));
                    }
                }
                title = Some((id, view));
            }
            let mut shown = Map::new();
            shown.insert("folder".to_owned(), json!(folder.display().to_string()));
            shown.insert("files".to_owned(), json!(files));
            shown.extend(fields);
            Ok((serde_json::Value::Object(shown), title))
        })
        .await?;
        let viewer = self.viewer_server().await?;
        viewer.show(shown.clone()).await;
        let link = viewer.link();
        let mut text = format!(
            "The 3D preview of the project in {} is at this link, for a web browser with \
             WebGPU:\n{link}\nDrag to turn it, scroll to zoom and click a part to see what it is; \
             the sliders open doors and drawers and pull the design apart. The link works while \
             this app stays connected to Stave.",
            project.folder.display()
        );
        if let Some((id, view)) = &title {
            let suggestion_title = &view.title;
            match &view.over {
                Over::Project => text.push_str(&format!(
                    "\nSuggestion {id} (\"{suggestion_title}\") is drawn as a ghost over the \
                     project as it is now, tinted by whether it checks clean."
                )),
                Over::Base { revision, .. } => text.push_str(&format!(
                    "\nSuggestion {id} (\"{suggestion_title}\") no longer fits the project as it \
                     is now, so the preview shows the project as it was when the suggestion was \
                     made (revision {revision}), with the suggestion drawn as a ghost over it, \
                     tinted by whether it checks clean. Changes made to the project since then \
                     aren't shown."
                )),
            }
        }
        let mut structured = shown;
        if let Some(fields) = structured.as_object_mut() {
            fields.insert("link".to_owned(), json!(link));
        }
        Ok(super::answer(text, structured))
    }

    async fn show_in_window(&self, suggestion: Option<String>) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let folder = project.folder.clone();
        let patch = match suggestion.clone() {
            Some(id) => Some(
                blocking(move || {
                    let history = agent_line::history(&folder)?;
                    let found = find_suggestion(&history, &id)?;
                    Ok(patch_file(&found.patches))
                })
                .await?,
            ),
            None => None,
        };
        match window_answer(stave_link::show_in_stave(&project.folder, patch.as_deref()).await)? {
            Shown::NotRunning => Err(McpError::StaveNotRunning),
            Shown::Opened => {
                let text = match &suggestion {
                    Some(id) => format!(
                        "Stave is showing the project in {} with suggestion {id} as a ghost. \
                         Nothing was written.",
                        project.folder.display()
                    ),
                    None => format!(
                        "Stave is showing the project in {}.",
                        project.folder.display()
                    ),
                };
                Ok(super::answer(
                    text,
                    json!({
                        "shown": true,
                        "folder": project.folder.display().to_string(),
                        "suggestion": suggestion,
                    }),
                ))
            }
        }
    }

    async fn picture(&self, camera: Camera) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        let (png, orbit) = blocking(move || {
            let opened = open(&project.folder)?;
            let design = opened.design.as_ref().ok_or(McpError::NotPlaced)?;
            let scene = stave_app::view::scene(design);
            let posed = scene.rest();
            let bounds = scene.bounds(&posed).ok_or(McpError::NothingToDraw)?;
            let framed =
                Orbit::framing(bounds, stave_render::aspect(PICTURE_WIDTH, PICTURE_HEIGHT));
            let orbit = camera.place(framed)?;
            let mut renderer = pollster::block_on(Renderer::headless()).map_err(render_error)?;
            renderer.upload(&scene);
            let image = renderer
                .render(&Frame {
                    scene: &scene,
                    posed: &posed,
                    orbit,
                    selection: &[],
                    highlight: None,
                    section: None,
                    looks: None,
                    overlay: None,
                    tint_cut_faces: true,
                    lit: None,
                    tints: None,
                    backdrop: Backdrop::Light,
                    width: PICTURE_WIDTH,
                    height: PICTURE_HEIGHT,
                })
                .map_err(render_error)?;
            let png = Renderer::encode_png(&image).map_err(render_error)?;
            Ok((png, orbit))
        })
        .await?;
        let yaw = orbit.yaw.to_degrees();
        let pitch = orbit.pitch.to_degrees();
        let text = format!(
            "A {PICTURE_WIDTH}x{PICTURE_HEIGHT} picture of the design, seen from {yaw:.0} degrees \
             around and {pitch:.0} degrees above, {distance:.0} mm away.",
            distance = orbit.distance
        );
        let mut result = CallToolResult::success(vec![
            ContentBlock::text(text),
            ContentBlock::image(
                base64::engine::general_purpose::STANDARD.encode(png),
                "image/png",
            ),
        ]);
        result.structured_content = Some(json!({
            "width": PICTURE_WIDTH,
            "height": PICTURE_HEIGHT,
            "camera": { "yaw": yaw, "pitch": pitch, "distance": orbit.distance },
        }));
        Ok(result)
    }
}

/// The window's answer in the server's words: with no session bus (a
/// headless or SSH session) the window can't be running there, so that
/// is said as plainly as a window that isn't open.
fn window_answer(answer: Result<Shown, LinkError>) -> Result<Shown, McpError> {
    match answer {
        Ok(shown) => Ok(shown),
        Err(LinkError::NoSessionBus(_)) => Err(McpError::StaveNotRunning),
        Err(other) => Err(McpError::ShowInStave(other)),
    }
}

/// `error` in the server's words: no adapter is said plainly.
fn render_error(error: RenderError) -> McpError {
    match error {
        RenderError::NoAdapter(_) | RenderError::NoDevice(_) => {
            McpError::NoGraphics(error.to_string())
        }
        other @ (RenderError::DeviceLost(_)
        | RenderError::Gpu(_)
        | RenderError::TooLarge { .. }
        | RenderError::Meshes(_)
        | RenderError::NotUploaded
        | RenderError::NoTarget
        | RenderError::Wait(_)
        | RenderError::Unfinished
        | RenderError::ReadBack(_)
        | RenderError::Mapping(_)
        | RenderError::NoSurface
        | RenderError::UnsuitableSurface
        | RenderError::Surface(_)
        | RenderError::Png(_)) => McpError::Render(other),
    }
}

/// Every file the project in `folder` reads, as the viewer takes them: a
/// file inside the folder by its path relative to it, one outside (a
/// library beside the project) by its full path. The standard library is
/// built into the viewer and left out.
fn project_files(folder: &Path) -> Result<BTreeMap<String, String>, McpError> {
    let opened = open(folder)?;
    Ok(opened
        .sources
        .files()
        .filter_map(|(_, source)| {
            let path = Path::new(source.path());
            if !path.is_absolute() {
                return None;
            }
            let key = path
                .strip_prefix(folder)
                .unwrap_or(path)
                .display()
                .to_string();
            Some((key, source.text().to_owned()))
        })
        .collect())
}

/// The suggestion `id` of the project's versions.
fn find_suggestion(
    history: &stave_history::History,
    id: &str,
) -> Result<stave_history::Suggestion, McpError> {
    let wanted = SuggestionId::new(id);
    history
        .suggestions()?
        .into_iter()
        .find(|suggestion| suggestion.id == wanted)
        .ok_or_else(|| McpError::NoSuchSuggestion(id.to_owned()))
}

/// What a previewed suggestion is drawn over.
enum Over {
    /// The project as it is in its folder now.
    Project,
    /// The project as the suggestion found it: it no longer fits the
    /// project as it is.
    Base {
        /// The revision on main the suggestion starts from.
        revision: RevisionId,
        /// The project's files at that revision.
        files: ProjectFiles,
    },
}

/// A suggestion as the preview shows it.
struct SuggestionView {
    /// The text files it changes or adds against what it is drawn over.
    changes: BTreeMap<String, String>,
    /// What it is drawn over.
    over: Over,
    /// Its title.
    title: String,
}

/// Suggestion `id` of the project in `folder`, laid over the project as
/// it is now: its own files when the project hasn't changed since it was
/// made, else its patches tried again on the project. When they no
/// longer fit it is laid over the revision it started from instead, so a
/// change made since is never shown as the suggestion taking it out.
fn suggestion_view(folder: &Path, id: &str) -> Result<SuggestionView, McpError> {
    let history = agent_line::history(folder)?;
    let suggestion = find_suggestion(&history, id)?;
    let base = history.files_at(&suggestion.base)?;
    let now = history.read_folder()?;
    if now == base {
        let after = history.suggestion_files(&suggestion.id)?;
        return Ok(SuggestionView {
            changes: changed_text(&base, &after),
            over: Over::Project,
            title: suggestion.title,
        });
    }
    // A project that doesn't load or a patch that no longer fits leaves
    // the revision the suggestion started from to draw it over.
    if let Ok(Replay::Fits { files, .. }) = agent_line::replay(folder, &now, &suggestion.patches) {
        return Ok(SuggestionView {
            changes: changed_text(&now, &files),
            over: Over::Project,
            title: suggestion.title,
        });
    }
    let after = history.suggestion_files(&suggestion.id)?;
    Ok(SuggestionView {
        changes: changed_text(&base, &after),
        over: Over::Base {
            revision: suggestion.base,
            files: base,
        },
        title: suggestion.title,
    })
}

/// The files of `after` that `before` doesn't hold as they are, those
/// that are text.
fn changed_text(before: &ProjectFiles, after: &ProjectFiles) -> BTreeMap<String, String> {
    after
        .iter()
        .filter(|(path, contents)| before.get(path) != Some(*contents))
        .filter_map(|(path, contents)| {
            std::str::from_utf8(contents)
                .ok()
                .map(|text| (path.to_string(), text.to_owned()))
        })
        .collect()
}

/// The text files of `files`, by their paths in the project folder.
fn text_files(files: &ProjectFiles) -> impl Iterator<Item = (String, String)> + '_ {
    files.iter().filter_map(|(path, contents)| {
        std::str::from_utf8(contents)
            .ok()
            .map(|text| (path.to_string(), text.to_owned()))
    })
}

/// A suggestion's patches as one patch file: one `stave` header, then
/// each patch in order.
fn patch_file(patches: &[String]) -> String {
    let bodies: Vec<&str> = patches.iter().map(|patch| without_header(patch)).collect();
    with_header(bodies.join("\n").trim_start())
}

/// `patch` without its `stave` header line, when it has one.
fn without_header(patch: &str) -> &str {
    let mut rest = patch;
    loop {
        let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            if after.is_empty() {
                return patch;
            }
            rest = after;
        } else if trimmed.starts_with("stave ") {
            return after;
        } else {
            return patch;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_goes_before_the_body_ends() {
        let page = with_script("<html><body><p>x</p></body></html>", "go()");
        assert_eq!(
            page,
            "<html><body><p>x</p><script>go()</script>\n</body></html>"
        );
        assert_eq!(with_script("<p>", "go()"), "<p><script>go()</script>\n");
    }

    #[test]
    fn the_app_page_speaks_the_bridge_and_calls_no_tool() {
        let page = app_page();
        assert!(page.contains("\"2026-01-26\""));
        assert!(page.contains("ui/notifications/initialized"));
        assert!(!page.contains("@PROTOCOL@") && !page.contains("@HEIGHT@"));
        assert!(!page.contains("tools/call"));
        assert!(local_page().contains("/scene?token="));
    }

    #[test]
    fn no_session_bus_is_stave_not_running() {
        let refused = window_answer(Err(LinkError::NoSessionBus(zbus::Error::Address(
            "no such socket".to_owned(),
        ))));
        let Err(error) = refused else {
            panic!("refused: {refused:?}");
        };
        assert!(matches!(error, McpError::StaveNotRunning), "{error:?}");
        assert_eq!(
            error.to_string(),
            "Stave isn't running. Open it and ask again."
        );
        assert!(matches!(
            window_answer(Ok(Shown::NotRunning)),
            Ok(Shown::NotRunning)
        ));
        assert!(matches!(
            window_answer(Err(LinkError::Bus(zbus::Error::Unsupported))),
            Err(McpError::ShowInStave(_))
        ));
    }

    #[test]
    fn patches_become_one_file_with_one_header() {
        let file = patch_file(&[
            "stave 1\n\npatch \"A\" {\n}\n".to_owned(),
            "// note\npatch \"B\" {\n}\n".to_owned(),
        ]);
        assert_eq!(file.matches("stave ").count(), 1, "{file}");
        assert!(file.starts_with("stave "));
        assert!(file.contains("patch \"A\"") && file.contains("// note\npatch \"B\""));
    }

    #[test]
    fn the_camera_is_checked() {
        let orbit = Orbit {
            yaw: 0.5,
            pitch: 0.25,
            distance: 2000.0,
            target: glam::Vec3::ZERO,
            projection: stave_scene::Projection::default(),
        };
        let bad = Camera {
            pitch: Some(90.5),
            ..Camera::default()
        };
        let error = bad.place(orbit).unwrap_err();
        assert!(error.to_string().contains("pitch"), "{error}");
        assert!(Camera::default().place(orbit).is_ok());
    }
}
