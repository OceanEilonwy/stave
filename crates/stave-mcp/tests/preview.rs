//! The preview: the localhost viewer's guards, the MCP App page the
//! `preview` tool points at, and `render_view`'s picture.

mod common;

use std::net::{IpAddr, Ipv4Addr};

use common::{connect, example_copy, is_error, options, structured};
use rmcp::model::{ReadResourceRequestParams, ResourceContents};
use serde_json::json;
use stave_mcp::viewer_server::ViewerServer;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;

/// Sends one raw HTTP/1.1 GET to the viewer and returns the status code
/// and body.
async fn get(server: &ViewerServer, target: &str, extra_headers: &[(&str, &str)]) -> (u16, String) {
    let address = server.address();
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut request = format!("GET {target} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in extra_headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8_lossy(&response).into_owned();
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {response:?}"));
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test(flavor = "multi_thread")]
async fn viewer_refuses_wrong_token() {
    let server = ViewerServer::start("<html><body>page</body></html>".into())
        .await
        .unwrap();
    let token = server.token().to_owned();
    assert_eq!(token.len(), 32);
    assert!(token.chars().all(|letter| letter.is_ascii_hexdigit()));

    assert_eq!(get(&server, "/", &[]).await.0, 403, "no token");
    assert_eq!(
        get(&server, "/?token=0123", &[]).await.0,
        403,
        "wrong token"
    );
    let wrong = "f".repeat(32);
    assert_eq!(get(&server, &format!("/?token={wrong}"), &[]).await.0, 403);
    assert_eq!(
        get(&server, &format!("/scene?token={wrong}"), &[]).await.0,
        403
    );

    let (status, body) = get(&server, &format!("/?token={token}"), &[]).await;
    assert_eq!(status, 200);
    assert!(body.contains("page"), "{body}");

    server
        .show(json!({ "folder": "/p", "files": { "box.stave": "stave 1\n" } }))
        .await;
    let (status, body) = get(&server, &format!("/scene?token={token}"), &[]).await;
    assert_eq!(status, 200);
    let scene: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(scene["project"]["files"]["box.stave"], "stave 1\n");
    assert_eq!(scene["version"], 1);

    assert_eq!(
        get(&server, &format!("/other?token={token}"), &[]).await.0,
        404
    );
    assert_eq!(get(&server, "/other", &[]).await.0, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn viewer_refuses_other_origin() {
    let server = ViewerServer::start("<html><body>page</body></html>".into())
        .await
        .unwrap();
    let token = server.token().to_owned();
    let page = format!("/?token={token}");
    let scene = format!("/scene?token={token}");
    for origin in [
        "http://evil.example",
        "null",
        "http://127.0.0.1:1",
        "https://claude.ai",
    ] {
        assert_eq!(
            get(&server, &page, &[("Origin", origin)]).await.0,
            403,
            "{origin}"
        );
        assert_eq!(
            get(&server, &scene, &[("Origin", origin)]).await.0,
            403,
            "{origin}"
        );
    }
    let own = format!("http://{}", server.address());
    assert_eq!(get(&server, &scene, &[("Origin", &own)]).await.0, 200);

    // A name that leads to the viewer through some other host's DNS is
    // refused too.
    let address = server.address();
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!(
        "GET {page} HTTP/1.1\r\nHost: evil.example:{}\r\nConnection: close\r\n\r\n",
        address.port()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 403"), "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn viewer_binds_loopback_only() {
    let server = ViewerServer::start("<html><body>page</body></html>".into())
        .await
        .unwrap();
    assert_eq!(server.address().ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_ne!(server.address().port(), 0);
    let link = server.link();
    assert!(
        link.starts_with(&format!(
            "http://127.0.0.1:{}/?token=",
            server.address().port()
        )),
        "{link}"
    );
    assert!(link.ends_with(server.token()));
    // The same port on this machine's own network address doesn't
    // answer: only the loopback address is listened on.
    let port = server.address().port();
    match network_address() {
        Some(address) => {
            let target = std::net::SocketAddr::new(address, port);
            assert!(
                TcpStream::connect(target).await.is_err(),
                "the viewer answers on {target}"
            );
        }
        None => eprintln!("no network address, only the loopback one was checked"),
    }
}

/// This machine's address on the network it would send to the outside
/// on (no packet is sent), when it has one that isn't loopback.
fn network_address() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;
    let address = socket.local_addr().ok()?.ip();
    (!address.is_loopback() && !address.is_unspecified()).then_some(address)
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_declares_ui_resource() {
    let connection = connect(options("preview_declares_ui_resource")).await;
    let tools = connection.client.list_all_tools().await.unwrap();
    let preview = tools
        .iter()
        .find(|tool| tool.name == "preview")
        .expect("a preview tool");
    let meta = serde_json::to_value(preview.meta.as_ref().expect("the tool's _meta")).unwrap();
    assert_eq!(meta["ui"]["resourceUri"], "ui://stave/viewer");

    let resources = connection.client.list_all_resources().await.unwrap();
    let viewer = resources
        .iter()
        .find(|resource| resource.uri == "ui://stave/viewer")
        .expect("the viewer resource");
    assert_eq!(
        viewer.mime_type.as_deref(),
        Some("text/html;profile=mcp-app")
    );

    let read = connection
        .client
        .read_resource(ReadResourceRequestParams::new("ui://stave/viewer"))
        .await
        .unwrap();
    let [contents] = read.contents.as_slice() else {
        panic!("one content: {read:?}");
    };
    let ResourceContents::TextResourceContents {
        uri,
        mime_type,
        text,
        ..
    } = contents
    else {
        panic!("text contents: {contents:?}");
    };
    assert_eq!(uri, "ui://stave/viewer");
    assert_eq!(mime_type.as_deref(), Some("text/html;profile=mcp-app"));
    // The page talks to its host through the Apps bridge, and hands the
    // tool result to the viewer.
    assert!(text.contains("ui/initialize"), "the bridge");
    assert!(text.contains("ui/notifications/tool-result"), "the bridge");
    assert!(text.contains("staveViewer"), "the viewer");
    // It never asks the host to call a tool.
    assert!(!text.contains("tools/call"));

    // preview on a project gives the files the page shows, and a link.
    let folder = example_copy("box", "preview_declares_ui_resource");
    let opened = connection
        .call(
            "open_project",
            json!({ "path": folder.display().to_string() }),
        )
        .await;
    assert!(!is_error(&opened), "{opened:?}");
    let result = connection.call("preview", json!({})).await;
    assert!(!is_error(&result), "{result:?}");
    let shown = structured(&result);
    assert_eq!(shown["folder"], folder.display().to_string());
    let files = shown["files"].as_object().unwrap();
    assert!(files.contains_key("box.stave"), "{files:?}");
    assert!(files.keys().all(|path| !path.starts_with('/')), "{files:?}");
    let link = shown["link"].as_str().unwrap();
    assert!(link.starts_with("http://127.0.0.1:"), "{link}");
    let text = common::text(&result);
    assert!(text.contains(link), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_built_viewer_is_embedded_when_built() {
    let built = common::workspace().join("target/viewer/stave-viewer.html");
    let connection = connect(options("the_built_viewer_is_embedded_when_built")).await;
    let read = connection
        .client
        .read_resource(ReadResourceRequestParams::new("ui://stave/viewer"))
        .await
        .unwrap();
    let Some(ResourceContents::TextResourceContents { text, .. }) = read.contents.first() else {
        panic!("text contents: {read:?}");
    };
    if built.exists() {
        // The WASM's glue and module are in the page.
        assert!(text.contains("wasm_bindgen"), "the glue is embedded");
        assert!(
            !text.contains("@STAVE_WASM_BASE64@"),
            "the module is embedded"
        );
        assert!(!text.contains("Viewer not built"));
    } else {
        assert!(text.contains("Viewer not built: run tools/build-viewer.sh"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn preview_needs_a_project() {
    let connection = connect(options("preview_needs_a_project")).await;
    for tool in ["preview", "render_view", "show_in_stave"] {
        let result = connection.call(tool, json!({})).await;
        assert!(is_error(&result), "{tool}: {result:?}");
        assert!(common::text(&result).contains("open_project"), "{tool}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn render_view_returns_png_or_says_why() {
    let connection = connect(options("render_view_returns_png_or_says_why")).await;
    let folder = example_copy("box", "render_view_returns_png_or_says_why");
    connection
        .call(
            "open_project",
            json!({ "path": folder.display().to_string() }),
        )
        .await;
    let refused = connection
        .call("render_view", json!({ "camera": { "pitch": 120.0 } }))
        .await;
    assert!(is_error(&refused), "{refused:?}");
    assert!(common::text(&refused).contains("pitch"));

    let result = connection
        .call(
            "render_view",
            json!({ "camera": { "yaw": 30.0, "pitch": 20.0 } }),
        )
        .await;
    if is_error(&result) {
        let text = common::text(&result);
        assert!(text.contains("graphics"), "says why: {text}");
        eprintln!("no GPU adapter: {text}");
        return;
    }
    let image = result
        .content
        .iter()
        .find_map(|content| content.as_image())
        .expect("an image");
    assert_eq!(image.mime_type, "image/png");
    use base64::Engine as _;
    let png = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(
        result
            .content
            .iter()
            .any(|content| content.as_text().is_some()),
        "a text that stands alone"
    );
}

const SHOE_SHELF: &str = r#"patch "Add a shoe shelf" {
  add part shoe_shelf : oak_19 {
    length: fit
    width:  cabinet_depth - back.thickness
    place {
      against(divider.top, gap: 400mm)
      center(base, across)
      flush(side_l.front)
    }
  } in "carcass.stave"
  add joint shoe_shelf_dados : dado(shoe_shelf, into: [side_l, side_r], depth: 9mm)
}
"#;

#[tokio::test(flavor = "multi_thread")]
async fn preview_shows_a_suggestion_over_the_project() {
    let test = "preview_shows_a_suggestion_over_the_project";
    let connection = connect(options(test)).await;
    let folder = example_copy("clothing_cabinet", test);
    let opened = connection
        .call(
            "open_project",
            json!({ "path": folder.display().to_string(), "start_versions": true }),
        )
        .await;
    assert!(!is_error(&opened), "{opened:?}");
    let suggested = connection
        .call("suggest", json!({ "patch": SHOE_SHELF, "title": "Shoes" }))
        .await;
    assert!(!is_error(&suggested), "{suggested:?}");
    let id = structured(&suggested)["id"].as_str().unwrap().to_owned();

    let result = connection
        .call("preview", json!({ "suggestion": id }))
        .await;
    assert!(!is_error(&result), "{result:?}");
    let shown = structured(&result);
    // Only what the suggestion changes comes as its files.
    let changes = shown["suggestion"].as_object().unwrap();
    assert_eq!(changes.keys().collect::<Vec<_>>(), ["carcass.stave"]);
    assert!(
        changes["carcass.stave"]
            .as_str()
            .unwrap()
            .contains("shoe_shelf")
    );
    assert!(
        !shown["files"]["carcass.stave"]
            .as_str()
            .unwrap()
            .contains("shoe_shelf")
    );
    assert!(common::text(&result).contains(&id));

    let missing = connection
        .call("preview", json!({ "suggestion": "no-such-suggestion" }))
        .await;
    assert!(is_error(&missing), "{missing:?}");
    assert!(common::text(&missing).contains("list_suggestions"));
}

/// An edit the person makes to `carcass.stave` after the suggestion.
const LATER_EDIT: &str = "\n// Edited by the person after the suggestion.\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_suggestion_is_drawn_over_the_project_as_it_is_now() {
    let test = "a_suggestion_is_drawn_over_the_project_as_it_is_now";
    let connection = connect(options(test)).await;
    let folder = example_copy("clothing_cabinet", test);
    let opened = connection
        .call(
            "open_project",
            json!({ "path": folder.display().to_string(), "start_versions": true }),
        )
        .await;
    assert!(!is_error(&opened), "{opened:?}");
    let suggested = connection
        .call("suggest", json!({ "patch": SHOE_SHELF, "title": "Shoes" }))
        .await;
    assert!(!is_error(&suggested), "{suggested:?}");
    let id = structured(&suggested)["id"].as_str().unwrap().to_owned();

    // The person edits the file the suggestion changes.
    let carcass = folder.join("carcass.stave");
    let mut text = std::fs::read_to_string(&carcass).unwrap();
    text.push_str(LATER_EDIT);
    std::fs::write(&carcass, &text).unwrap();

    let result = connection
        .call("preview", json!({ "suggestion": id }))
        .await;
    assert!(!is_error(&result), "{result:?}");
    let shown = structured(&result);
    assert_eq!(shown["suggestion_over"], "project", "{shown}");
    let project = shown["files"]["carcass.stave"].as_str().unwrap();
    assert!(
        project.contains(LATER_EDIT.trim()),
        "the project as it is now"
    );
    let ghost = shown["suggestion"]["carcass.stave"].as_str().unwrap();
    // The ghost keeps the person's edit: it isn't shown as taken out.
    assert!(ghost.contains("shoe_shelf"), "{ghost}");
    assert!(ghost.contains(LATER_EDIT.trim()), "{ghost}");
    assert!(common::text(&result).contains("as it is now"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_suggestion_that_no_longer_fits_is_drawn_over_its_base() {
    let test = "a_suggestion_that_no_longer_fits_is_drawn_over_its_base";
    let connection = connect(options(test)).await;
    let folder = example_copy("clothing_cabinet", test);
    let opened = connection
        .call(
            "open_project",
            json!({ "path": folder.display().to_string(), "start_versions": true }),
        )
        .await;
    assert!(!is_error(&opened), "{opened:?}");
    let suggested = connection
        .call("suggest", json!({ "patch": SHOE_SHELF, "title": "Shoes" }))
        .await;
    assert!(!is_error(&suggested), "{suggested:?}");
    let id = structured(&suggested)["id"].as_str().unwrap().to_owned();

    // The person renames the divider the shoe shelf is placed against,
    // so the suggestion's patch no longer fits.
    let carcass = folder.join("carcass.stave");
    let before = std::fs::read_to_string(&carcass).unwrap();
    let after = before.replace("divider", "partition");
    assert_ne!(before, after);
    std::fs::write(&carcass, &after).unwrap();

    let result = connection
        .call("preview", json!({ "suggestion": id }))
        .await;
    assert!(!is_error(&result), "{result:?}");
    let shown = structured(&result);
    assert_eq!(shown["suggestion_over"], "base", "{shown}");
    assert!(shown["suggestion_base"].is_string(), "{shown}");
    // The project is drawn as the suggestion found it, not with the
    // rename, so the ghost shows only what the suggestion changes.
    assert_eq!(shown["files"]["carcass.stave"].as_str().unwrap(), before);
    let ghost = shown["suggestion"]["carcass.stave"].as_str().unwrap();
    assert!(ghost.contains("shoe_shelf") && ghost.contains("divider"));
    let text = common::text(&result);
    assert!(text.contains("no longer fits"), "{text}");
}

/// The Content Security Policy the MCP Apps spec (2026-01-26) gives a UI
/// resource that declares no domains: it has no `'wasm-unsafe-eval'`.
const APPS_DEFAULT_CSP: &str = "default-src 'none'; script-src 'self' 'unsafe-inline'; \
                                style-src 'self' 'unsafe-inline'; img-src 'self' data:; \
                                media-src 'self' data:; connect-src 'none'";

/// What the page says when its host won't let it compile WebAssembly.
const WASM_BLOCKED: &str = "This app can't run the 3D preview here. Open the link from the \
                            preview's answer in a web browser.";

/// A Chromium to load pages in: `STAVE_CHROMIUM`, or one on `PATH`.
fn chromium() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("STAVE_CHROMIUM") {
        return Some(path.into());
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|folder| {
            [
                "chromium",
                "chromium-browser",
                "google-chrome",
                "google-chrome-stable",
            ]
            .map(|name| folder.join(name))
        })
        .find(|candidate| candidate.is_file())
}

/// Serves `page` with `csp` as its Content-Security-Policy header to
/// every request, on a loopback port; returns the address.
async fn serve_with_csp(page: String, csp: &'static str) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let page = std::sync::Arc::new(page);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let page = page.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                     Content-Security-Policy: {csp}\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    page.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(page.as_bytes()).await;
            });
        }
    });
    address
}

/// The MCP App page under the host's default Content Security Policy:
/// Chromium (which Claude Desktop is built on) refuses to compile its
/// WebAssembly, and the page says so in plain words instead of staying
/// blank or showing the browser's error.
#[tokio::test(flavor = "multi_thread")]
async fn the_app_page_says_why_under_the_apps_default_csp() {
    let page = stave_mcp::tools::preview::app_page();
    if page.contains("Viewer not built") {
        eprintln!("the viewer isn't built (run tools/build-viewer.sh); skipped");
        return;
    }
    let Some(chromium) = chromium() else {
        eprintln!("no Chromium on PATH (or STAVE_CHROMIUM); skipped");
        return;
    };
    let address = serve_with_csp(page, APPS_DEFAULT_CSP).await;
    let profile = common::scratch_folder("the_app_page_says_why_under_the_apps_default_csp");
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(chromium)
            .arg("--headless")
            .arg("--no-sandbox")
            .arg("--no-first-run")
            .arg("--enable-unsafe-webgpu")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--virtual-time-budget=10000")
            .arg("--dump-dom")
            .arg(format!("http://{address}/"))
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    let dom = String::from_utf8_lossy(&output.stdout);
    let message = dom
        .split_once("<p id=\"message\"")
        .and_then(|(_, rest)| rest.split_once("</p>"))
        .map(|(message, _)| message)
        .unwrap_or_else(|| {
            panic!(
                "no message in the page; Chromium said: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        });
    assert!(
        message.contains(WASM_BLOCKED),
        "the page says why it can't draw: {message}"
    );
    assert!(!message.contains("hidden"), "the message shows: {message}");
}
