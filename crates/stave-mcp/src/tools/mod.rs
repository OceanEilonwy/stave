//! The tools, one module per group. Each group is a `#[tool_router]`
//! block on [`StaveServer`] with its own router; [`router`] joins them,
//! one line per group, and a new group registers itself there.
//!
//! Every tool answers with one text content that stands on its own and
//! the same facts as structured content ([`answer`]), or, when it can't
//! do what was asked, with a plain-words refusal marked as an error
//! ([`refusal`]).

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value, json};

use crate::{McpError, StaveServer};

pub mod changing;
pub mod conflicts;
pub mod looking;
pub mod projects;
pub mod revisions;

/// Every tool the server offers.
pub(crate) fn router() -> ToolRouter<StaveServer> {
    StaveServer::projects_router()
        + StaveServer::looking_router()
        + StaveServer::changing_router()
        + StaveServer::revisions_router()
        + StaveServer::conflicts_router()
}

/// A tool's answer: `text` for a person to read, `structured` for a
/// program.
pub fn answer(text: String, structured: Value) -> CallToolResult {
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    result
}

/// A tool's refusal: what went wrong, in plain words, as both the text
/// and `{"error": ...}`.
pub fn refusal(error: &McpError) -> CallToolResult {
    let text = error.to_string();
    let mut result = CallToolResult::error(vec![ContentBlock::text(text.clone())]);
    result.structured_content = Some(json!({ "error": text }));
    result
}

/// The answer, or the refusal, of a tool's work.
pub fn finish(outcome: Result<CallToolResult, McpError>) -> CallToolResult {
    outcome.unwrap_or_else(|error| refusal(&error))
}

/// Runs blocking work (opening and checking a project) off the
/// connection's thread.
pub async fn blocking<T, F>(work: F) -> Result<T, McpError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, McpError> + Send + 'static,
{
    tokio::task::spawn_blocking(work).await?
}
