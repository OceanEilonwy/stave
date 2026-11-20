//! The localhost viewer: the preview page for AI apps that can't show
//! one themselves (Claude Code, Codex), served to the person's own
//! browser.
//!
//! ```text
//! preview ──show(project)──▶ ViewerServer (127.0.0.1:<random port>)
//!                              ├── GET /?token=…       the page
//!                              └── GET /scene?token=…  {version, project}
//! browser ◀──────────── the page polls /scene and draws what it holds
//! ```
//!
//! The server listens on the loopback address only, on a port the system
//! picks, and answers only a request that carries the session's token
//! (32 hex digits from the system's random source) and comes from its own
//! origin: a wrong or missing token, an `Origin` that isn't the server's
//! own or a `Host` that isn't its loopback address is refused with 403,
//! so another web page in the same browser can neither read the project
//! nor reach it through a name of its own. Any other path or method is
//! 404. It serves what the last `preview` showed and nothing else; it
//! reads no file and changes nothing.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::Response;
use serde_json::{Value, json};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

/// Why the viewer couldn't be started.
#[derive(Debug, thiserror::Error)]
pub enum ViewerError {
    /// The system's random source gave no token.
    #[error("no random token for the viewer: {0}")]
    Random(getrandom::Error),
    /// The loopback address couldn't be listened on.
    #[error("the viewer can't listen on 127.0.0.1: {0}")]
    Listen(#[source] std::io::Error),
}

/// A session's token: 32 lower-case hex digits.
#[derive(Clone, PartialEq, Eq)]
struct Token(String);

impl Token {
    /// A fresh token from the system's random source.
    fn new() -> Result<Self, ViewerError> {
        let mut bytes = [0_u8; 16];
        getrandom::fill(&mut bytes).map_err(ViewerError::Random)?;
        Ok(Self(
            bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
        ))
    }

    /// Whether `given` is this token, compared in time that doesn't
    /// depend on where they first differ.
    fn matches(&self, given: &str) -> bool {
        let (ours, theirs) = (self.0.as_bytes(), given.as_bytes());
        ours.len() == theirs.len()
            && ours
                .iter()
                .zip(theirs)
                .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
                == 0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Kept out of logs.
        formatter.write_str("Token(..)")
    }
}

/// What `/scene` serves: the project the last `preview` showed, and how
/// many times one was shown, so the page knows when to draw again.
#[derive(Debug, Default)]
struct Scene {
    version: u64,
    project: Option<Value>,
}

/// What the request handler shares.
#[derive(Debug)]
struct Shared {
    token: Token,
    port: u16,
    page: Arc<str>,
    scene: RwLock<Scene>,
}

/// The localhost viewer, serving until it is dropped.
#[derive(Debug)]
pub struct ViewerServer {
    address: SocketAddr,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl ViewerServer {
    /// Starts serving `page` on `127.0.0.1` at a port the system picks,
    /// with a fresh token. Needs a Tokio runtime.
    ///
    /// # Errors
    ///
    /// When no token can be made or the address can't be listened on.
    pub async fn start(page: Arc<str>) -> Result<Self, ViewerError> {
        let token = Token::new()?;
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(ViewerError::Listen)?;
        let address = listener.local_addr().map_err(ViewerError::Listen)?;
        let shared = Arc::new(Shared {
            token,
            port: address.port(),
            page,
            scene: RwLock::new(Scene::default()),
        });
        let app = Router::new()
            .fallback(answer)
            .with_state(Arc::clone(&shared));
        let task = tokio::spawn(async move {
            // It stops only when the listener fails, and then the
            // preview's link stops working; there is no one to tell.
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self {
            address,
            shared,
            task,
        })
    }

    /// The address it listens on: `127.0.0.1` and its port.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The session's token.
    pub fn token(&self) -> &str {
        &self.shared.token.0
    }

    /// The page's link, token included.
    pub fn link(&self) -> String {
        format!("http://{}/?token={}", self.address, self.shared.token.0)
    }

    /// Serves `project` (`{folder, files, suggestion}`, what the page
    /// hands `staveViewer.show`) from now on.
    pub async fn show(&self, project: Value) {
        let mut scene = self.shared.scene.write().await;
        scene.version = scene.version.saturating_add(1);
        scene.project = Some(project);
    }
}

impl Drop for ViewerServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Answers every request: the guards first, then the page or the scene.
async fn answer(
    State(shared): State<Arc<Shared>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    if !from_own_origin(&shared, &headers) {
        return status(StatusCode::FORBIDDEN, "Forbidden");
    }
    let wanted = match (method, uri.path()) {
        (Method::GET, "/") => Wanted::Page,
        (Method::GET, "/scene") => Wanted::Scene,
        _ => return status(StatusCode::NOT_FOUND, "Not found"),
    };
    let token = uri.query().and_then(|query| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix("token="))
    });
    if !token.is_some_and(|token| shared.token.matches(token)) {
        return status(StatusCode::FORBIDDEN, "Forbidden");
    }
    match wanted {
        Wanted::Page => respond(
            StatusCode::OK,
            "text/html; charset=utf-8",
            shared.page.to_string(),
        ),
        Wanted::Scene => {
            let scene = shared.scene.read().await;
            let body = json!({ "version": scene.version, "project": scene.project });
            respond(StatusCode::OK, "application/json", body.to_string())
        }
    }
}

/// What a request asks for.
enum Wanted {
    Page,
    Scene,
}

/// Whether the request's `Host` (when given) is the server's loopback
/// address and its `Origin` (when given) the server's own.
fn from_own_origin(shared: &Shared, headers: &HeaderMap) -> bool {
    let port = shared.port;
    let own_host = |value: &HeaderValue| {
        value.to_str().is_ok_and(|host| {
            host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
        })
    };
    let own_origin = |value: &HeaderValue| {
        value.to_str().is_ok_and(|origin| {
            origin == format!("http://127.0.0.1:{port}")
                || origin == format!("http://localhost:{port}")
        })
    };
    headers.get_all(header::HOST).iter().all(own_host)
        && headers.get_all(header::ORIGIN).iter().all(own_origin)
}

/// A plain-text answer with `code`.
fn status(code: StatusCode, text: &'static str) -> Response {
    respond(code, "text/plain; charset=utf-8", text.to_owned())
}

/// An answer with `code`, `content_type` and `body`, never cached and
/// never passing the link (and its token) on as a referrer.
fn respond(code: StatusCode, content_type: &'static str, body: String) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = code;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_32_hex_digits_and_differ() {
        let first = Token::new().unwrap();
        let second = Token::new().unwrap();
        assert_eq!(first.0.len(), 32);
        assert!(first.0.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
        assert!(first.matches(&first.0.clone()));
        assert!(!first.matches(&second.0));
        assert!(!first.matches(""));
        assert!(!first.matches(&first.0[..31]));
    }
}
