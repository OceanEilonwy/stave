//! Changing: `try_patch`, `fixes_for`. Neither writes anything: a patch
//! is tried on a scratch copy of the project, the way `stave edit
//! --patch` tries one, and a fix is itself a patch to try.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::AtomicBool;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stave_app::edit::Verdict;
use stave_app::scratch::{EditBase, one_patch_each};
use stave_lang::{Parse, SourceMap, ast, parse_patch_file, render_diagnostics};
use stave_modules::DiskFiles;

use super::looking::{count_words, problem_json};
use super::projects::{load_error, open};
use super::{answer, blocking, finish};
use crate::{McpError, StaveServer};

/// The name a patch's own diagnostics point into.
const PATCH_NAME: &str = "patch";

/// `try_patch`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchArguments {
    /// One or more patches in the .stave patch grammar (see grammar
    /// "Patches"): `patch "Wider" { set cabinet_width = 950mm }`.
    pub patch: String,
}

/// `fixes_for`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FixesArguments {
    /// The problem's number in check's list, counted from 1.
    pub problem: usize,
}

#[tool_router(router = changing_router, vis = "pub(crate)")]
impl StaveServer {
    /// Tries a patch on a scratch copy.
    #[tool(
        description = "Try one or more patches on a scratch copy of the open project, in order, \
                       each on top of the ones before it, and stop at the first that is \
                       refused. Says for each whether it fits (checks with no new errors) or is \
                       refused and why, with the problems it causes and the files it would \
                       change. Writes nothing.",
        annotations(read_only_hint = true)
    )]
    async fn try_patch(&self, Parameters(arguments): Parameters<PatchArguments>) -> CallToolResult {
        finish(self.try_patches(arguments.patch).await)
    }

    /// A problem's fixes.
    #[tool(
        description = "The fixes of one of the open project's problems, by its number in \
                       check's list: each a patch, ready for try_patch.",
        annotations(read_only_hint = true)
    )]
    async fn fixes_for(&self, Parameters(arguments): Parameters<FixesArguments>) -> CallToolResult {
        finish(self.fixes(arguments.problem).await)
    }
}

impl StaveServer {
    async fn try_patches(&self, text: String) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        blocking(move || try_on_scratch(&project.folder, &text)).await
    }

    async fn fixes(&self, number: usize) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        blocking(move || {
            let opened = open(&project.folder)?;
            let count = opened.diagnostics.len();
            let diagnostic = number
                .checked_sub(1)
                .and_then(|index| opened.diagnostics.get(index))
                .ok_or(McpError::NoSuchProblem { number, count })?;
            let mut text = format!(
                "Problem {number}: {}[{}]: {}\n",
                diagnostic.severity.word(),
                diagnostic.code.as_str(),
                diagnostic.message
            );
            if diagnostic.fixes.is_empty() {
                text.push_str("It has no fixes: putting it right needs a judgement.");
                if let Some(help) = &diagnostic.help {
                    let _ = write!(text, " Help: {help}");
                }
            } else {
                text.push_str("Fixes, the recommended one first; try one with try_patch:\n");
                for fix in &diagnostic.fixes {
                    let _ = writeln!(text, "  {fix}");
                }
            }
            let problem = problem_json(&opened.sources, &project.folder, diagnostic);
            let fixes = problem.get("fixes").cloned().unwrap_or(Value::Null);
            Ok(answer(
                text,
                json!({ "number": number, "problem": problem, "fixes": fixes }),
            ))
        })
        .await
    }
}

/// Tries each patch of `text` on a scratch copy of the project in
/// `folder`, as `stave edit --patch` does, and says what happened.
fn try_on_scratch(folder: &Path, text: &str) -> Result<CallToolResult, McpError> {
    let text = with_header(text);
    let mut sources = SourceMap::new();
    let file = sources.add(PATCH_NAME, text.as_str())?;
    let patches = one_patch_each(file, &parse_patch_file(file, &text));
    let mut base = EditBase::new(folder, &DiskFiles).map_err(|error| load_error(folder, error))?;
    let cancel = AtomicBool::new(false);
    let mut words = String::new();
    let mut tried_patches = Vec::new();
    let (mut fit, mut refused) = (0_usize, 0_usize);
    for patch in &patches {
        let tried = base.try_patch(PATCH_NAME, patch, &cancel)?;
        let title = title(patch);
        let fits = match tried.verdict() {
            Verdict::Fits(_) => true,
            Verdict::Refused(_) | Verdict::Checking => false,
        };
        let _ = writeln!(words, "{title}: {}", if fits { "fits" } else { "refused" });
        let mut changed = Vec::new();
        let mut created = Vec::new();
        if fits {
            if tried.changed().is_empty() {
                words.push_str("  would change nothing\n");
            }
            for file in tried.changed() {
                let shown = file
                    .path
                    .strip_prefix(folder)
                    .unwrap_or(&file.path)
                    .display()
                    .to_string();
                let verb = if file.created { "create" } else { "change" };
                let _ = writeln!(words, "  would {verb} {shown}");
                if file.created {
                    created.push(shown.clone());
                }
                changed.push(shown);
            }
        }
        let shown = tried.shown();
        words.push_str(&render_diagnostics(tried.sources(), &shown));
        let problems: Vec<Value> = shown
            .iter()
            .map(|diagnostic| problem_json(tried.sources(), folder, diagnostic))
            .collect();
        tried_patches.push(json!({
            "title": title,
            "verdict": if fits { "fits" } else { "refused" },
            "changed": changed,
            "created": created,
            "problems": problems,
        }));
        if fits {
            fit = fit.saturating_add(1);
            base.advance(tried)?;
        } else {
            refused = refused.saturating_add(1);
            break;
        }
    }
    let _ = writeln!(
        words,
        "checked {} of {}: {fit} {}, {refused} refused. Nothing was written.",
        fit.saturating_add(refused),
        count_words(patches.len(), "patch", "patches"),
        if fit == 1 { "fits" } else { "fit" },
    );
    Ok(answer(
        words,
        json!({ "fits": refused == 0, "patches": tried_patches }),
    ))
}

/// The patch text as a patch file: a file starts with its format line,
/// `stave 1`, which an AI writing a patch by itself leaves out, so it is
/// added when missing (the patch's own line numbers then count it).
fn with_header(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("//"));
    if first.is_some_and(|line| line.starts_with("stave ")) {
        text.to_owned()
    } else {
        format!("stave {}\n\n{text}", stave_lang::FORMAT_VERSION)
    }
}

/// How a patch is named: `patch "Add a shelf"`, or `the patch text` when
/// it isn't one readable patch.
fn title(patch: &Parse<ast::PatchFile>) -> String {
    let only = match patch.ast.as_ref().map(|ast| ast.patches.as_slice()) {
        Some([only]) if !patch.has_errors() => only,
        Some(_) | None => return "the patch text".to_owned(),
    };
    // The string as written, quotes included.
    let text = patch.syntax().to_string();
    let span = only.summary.span;
    let written = usize::try_from(span.start())
        .ok()
        .zip(usize::try_from(span.end()).ok())
        .and_then(|(start, end)| text.get(start..end))
        .unwrap_or("\"\"");
    format!("patch {written}")
}
