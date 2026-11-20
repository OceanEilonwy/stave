//! Shared helpers for the server's tests: a server and an in-process
//! client joined by a duplex stream, temporary folders, examples copied.

#![allow(dead_code, reason = "each test file uses some of the helpers")]

use std::path::{Path, PathBuf};

use rmcp::ServiceExt as _;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
};
use rmcp::service::{RoleClient, RunningService};
use serde_json::Value;
use stave_mcp::{ServerOptions, StaveServer};

/// The repository's root.
pub fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh, empty temporary folder named for the test.
pub fn scratch_folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("stave-mcp-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Copies `examples/<example>` into a fresh temporary folder and returns
/// the copy's path.
pub fn example_copy(example: &str, test: &str) -> PathBuf {
    copy_of(&format!("examples/{example}"), test)
}

/// Copies a folder of the repository (`tests/broken/...`) into a fresh
/// temporary folder and returns the copy's path, canonical.
pub fn copy_of(relative: &str, test: &str) -> PathBuf {
    let name = Path::new(relative).file_name().unwrap();
    let folder = scratch_folder(test).canonicalize().unwrap().join(name);
    copy_folder(&workspace().join(relative), &folder);
    folder
}

fn copy_folder(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_folder(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// Every file under `folder` with its bytes, sorted by path.
pub fn snapshot(folder: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    collect(folder, &mut files);
    files.sort();
    files
}

fn collect(folder: &Path, files: &mut Vec<(PathBuf, Vec<u8>)>) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            let bytes = std::fs::read(&path).unwrap();
            files.push((path, bytes));
        }
    }
}

/// A connected client, with the server running beside it.
pub struct Connection {
    /// The client side.
    pub client: RunningService<RoleClient, ClientConfig>,
}

/// Options whose folders are inside a test's temporary folder.
pub fn options(test: &str) -> ServerOptions {
    let home = scratch_folder(&format!("{test}-home"));
    ServerOptions {
        config_dir: home.join("config"),
        cache_dir: home.join("cache"),
        recent_projects: home.join("data/recently-used.xbel"),
    }
}

/// Starts a server with `options` and connects a client named "Test Host".
pub async fn connect(options: ServerOptions) -> Connection {
    let (server_transport, client_transport) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = StaveServer::new(options).serve(server_transport).await {
            let _ = running.waiting().await;
        }
    });
    let client = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("Test Host", "1.0"),
    )
    .serve(client_transport)
    .await
    .unwrap();
    Connection { client }
}

impl Connection {
    /// Calls a tool with JSON arguments.
    pub async fn call(&self, tool: &'static str, arguments: Value) -> CallToolResult {
        let mut request = CallToolRequestParams::new(tool);
        if let Value::Object(map) = arguments {
            request = request.with_arguments(map);
        }
        self.client.call_tool(request).await.unwrap()
    }
}

/// The text of a tool's result: its one text content.
pub fn text(result: &CallToolResult) -> String {
    assert_eq!(result.content.len(), 1, "one content: {result:?}");
    result.content[0]
        .as_text()
        .map(|text| text.text.clone())
        .expect("text content")
}

/// The structured content of a tool's result.
pub fn structured(result: &CallToolResult) -> &Value {
    result
        .structured_content
        .as_ref()
        .expect("structured content")
}

/// Whether the result is an error.
pub fn is_error(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}
