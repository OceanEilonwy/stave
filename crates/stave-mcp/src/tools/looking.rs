//! Looking: `check`, `describe`, `grammar`, `std_lookup`. None of them
//! changes anything.
//!
//! `grammar` and `std_lookup` read the language guide (`docs/grammar.md`)
//! and the standard modules (`std/`) as built into Stave, so what the AI
//! reads is what this Stave understands.

use std::fmt::Write as _;
use std::path::Path;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stave_lang::{Diagnostic, Severity, SourceMap, render_diagnostics};
use stave_model::solved::{AssemblyId, SolvedDesign};
use stave_scene::Item;

use super::projects::open;
use super::{answer, blocking, finish};
use crate::{McpError, StaveServer};

/// The language guide, as built into Stave.
const GRAMMAR: &str = include_str!("../../../../docs/grammar.md");

/// `describe`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeArguments {
    /// A part or piece of hardware by its name as written
    /// (`door_l.stile[hinge]`); leave it out for the whole design.
    pub path: Option<String>,
}

/// `grammar`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GrammarArguments {
    /// A section of the language guide by its title ("Patches",
    /// "Regions", "Joints and hardware"); leave it out for the list of
    /// sections.
    pub topic: Option<String>,
}

/// `std_lookup`'s arguments.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupArguments {
    /// The name of a kind (or recipe or profile) of the standard modules.
    pub kind: String,
}

#[tool_router(router = looking_router, vis = "pub(crate)")]
impl StaveServer {
    /// The open project's problems.
    #[tool(
        description = "Check the open project: every problem as `stave check` prints it, with \
                       file, line, a help line and fixes (each fix is a patch). Problems are \
                       numbered in the structured result for fixes_for.",
        annotations(read_only_hint = true)
    )]
    async fn check(&self) -> CallToolResult {
        finish(self.check_project().await)
    }

    /// The parts tree, or one item.
    #[tool(
        description = "Describe the open project's design: without a path, every assembly with \
                       its parts and hardware; with a path (a part's or hardware's name), what \
                       the window's inspector says about it: stock, sizes, grain, assembly.",
        annotations(read_only_hint = true)
    )]
    async fn describe(
        &self,
        Parameters(arguments): Parameters<DescribeArguments>,
    ) -> CallToolResult {
        finish(self.describe_design(arguments.path).await)
    }

    /// A section of the language guide.
    #[tool(
        description = "Read a section of the .stave language guide by its title, such as \
                       \"Patches\" (how to write a change), \"Placement\", \"Regions\" or \
                       \"Joints and hardware\"; without a topic, the list of sections.",
        annotations(read_only_hint = true)
    )]
    async fn grammar(&self, Parameters(arguments): Parameters<GrammarArguments>) -> CallToolResult {
        finish(match arguments.topic {
            None => Ok(contents()),
            Some(topic) => section(&topic),
        })
    }

    /// A kind of the standard modules.
    #[tool(
        description = "Look up a joint or hardware kind (or recipe or profile) of Stave's \
                       standard modules by name: its whole declaration with its documentation, \
                       settings and roles, and the module to import it from.",
        annotations(read_only_hint = true)
    )]
    async fn std_lookup(
        &self,
        Parameters(arguments): Parameters<LookupArguments>,
    ) -> CallToolResult {
        finish(lookup(&arguments.kind))
    }
}

impl StaveServer {
    async fn check_project(&self) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        blocking(move || {
            let opened = open(&project.folder)?;
            let count = |severity: Severity| {
                opened
                    .diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.severity == severity)
                    .count()
            };
            let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
            let text = format!(
                "{}checked {}: {}, {}\n",
                render_diagnostics(&opened.sources, &opened.diagnostics),
                plain_count(opened.root_files, "file", "files"),
                plain_count(errors, "error", "errors"),
                plain_count(warnings, "warning", "warnings"),
            );
            let problems: Vec<Value> = opened
                .diagnostics
                .iter()
                .enumerate()
                .map(|(index, diagnostic)| {
                    let mut problem = problem_json(&opened.sources, &project.folder, diagnostic);
                    if let Some(fields) = problem.as_object_mut() {
                        fields.insert("number".to_owned(), json!(index.saturating_add(1)));
                    }
                    problem
                })
                .collect();
            Ok(answer(
                text,
                json!({
                    "files": opened.root_files,
                    "errors": errors,
                    "warnings": warnings,
                    "problems": problems,
                }),
            ))
        })
        .await
    }

    async fn describe_design(&self, path: Option<String>) -> Result<CallToolResult, McpError> {
        let project = self.require_project().await?;
        blocking(move || {
            let opened = open(&project.folder)?;
            let design = opened.design.as_ref().ok_or(McpError::NotPlaced)?;
            match path {
                None => Ok(tree(design)),
                Some(name) => item(design, &name),
            }
        })
        .await
    }
}

/// `n things`, as `stave check` counts: `0 errors`, `1 error`.
fn plain_count(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// `n things` in words: `no errors`, `1 error`, `3 errors`.
pub(crate) fn count_words(count: usize, one: &str, many: &str) -> String {
    match count {
        0 => format!("no {many}"),
        1 => format!("1 {one}"),
        _ => format!("{count} {many}"),
    }
}

/// A diagnostic as structured content: severity, code, message, where
/// (relative to the project folder), help and fixes.
pub(crate) fn problem_json(sources: &SourceMap, folder: &Path, diagnostic: &Diagnostic) -> Value {
    let location = diagnostic.primary().and_then(|label| {
        let file = sources.get(label.file)?;
        let at = file.line_column(label.span.start());
        let path = Path::new(file.path());
        let shown = path.strip_prefix(folder).unwrap_or(path);
        Some(json!({
            "file": shown.display().to_string(),
            "line": at.line,
            "column": at.column,
        }))
    });
    let fixes: Vec<Value> = diagnostic
        .fixes
        .iter()
        .map(|fix| json!({ "label": fix.label(), "patch": fix.to_string() }))
        .collect();
    json!({
        "severity": diagnostic.severity.word(),
        "code": diagnostic.code.as_str(),
        "message": diagnostic.message,
        "location": location,
        "notes": diagnostic.notes,
        "help": diagnostic.help,
        "fixes": fixes,
    })
}

/// Every assembly with its parts and hardware, nested as in the design.
fn tree(design: &SolvedDesign) -> CallToolResult {
    let mut text = format!(
        "\"{}\": {} and {}.\n",
        design.project.title,
        count_words(design.parts().len(), "part", "parts"),
        count_words(
            design.hardware().len(),
            "piece of hardware",
            "pieces of hardware"
        ),
    );
    text.push_str("The project itself\n");
    contents_of(design, None, 1, &mut text);
    let parts: Vec<Value> = design
        .parts()
        .iter()
        .map(|part| {
            json!({
                "name": part.name,
                "stock": part.stock.name,
                "assembly": part.assembly.and_then(|id| design.assembly(id)).map(|a| &a.name),
            })
        })
        .collect();
    let hardware: Vec<Value> = design
        .hardware()
        .iter()
        .map(|hardware| {
            json!({
                "name": hardware.name,
                "kind": hardware.kind,
                "assembly": hardware.assembly.and_then(|id| design.assembly(id)).map(|a| &a.name),
            })
        })
        .collect();
    let assemblies: Vec<Value> = design
        .assemblies()
        .iter()
        .map(|assembly| {
            json!({
                "name": assembly.name,
                "component": assembly.component,
                "parent": assembly.parent.and_then(|id| design.assembly(id)).map(|a| &a.name),
            })
        })
        .collect();
    answer(
        text,
        json!({
            "title": design.project.title,
            "parts": parts,
            "hardware": hardware,
            "assemblies": assemblies,
        }),
    )
}

/// The parts, hardware and nested assemblies directly in `assembly`
/// (`None`: the project itself), indented `depth` steps.
fn contents_of(
    design: &SolvedDesign,
    assembly: Option<AssemblyId>,
    depth: usize,
    text: &mut String,
) {
    let indent = "  ".repeat(depth);
    for part in design
        .parts()
        .iter()
        .filter(|part| part.assembly == assembly)
    {
        let description = stave_app::view::describe(design, Item::Part(part.id));
        let row = |label: &str| {
            description
                .as_ref()
                .and_then(|description| description.rows.iter().find(|row| row.label == label))
                .map_or("", |row| row.value.as_str())
        };
        let _ = writeln!(
            text,
            "{indent}part {}: {}, cut {}",
            part.name,
            row("Stock"),
            row("Cut size")
        );
    }
    for hardware in design
        .hardware()
        .iter()
        .filter(|hardware| hardware.assembly == assembly)
    {
        let _ = writeln!(text, "{indent}{} {}", hardware.kind, hardware.name);
    }
    // An assembly's parent always comes before it, so this nests at most
    // as deep as the design has assemblies.
    for child in design
        .assemblies()
        .iter()
        .filter(|child| child.parent == assembly)
    {
        let _ = writeln!(
            text,
            "{indent}assembly {} ({})",
            child.name, child.component
        );
        if depth <= design.assemblies().len() {
            contents_of(design, Some(child.id), depth.saturating_add(1), text);
        }
    }
}

/// What the inspector says about the part or hardware called `name`.
fn item(design: &SolvedDesign, name: &str) -> Result<CallToolResult, McpError> {
    let found = design
        .part_named(name)
        .map(|part| Item::Part(part.id))
        .or_else(|| {
            design
                .hardware()
                .iter()
                .find(|hardware| hardware.name == name)
                .map(|hardware| Item::Hardware(hardware.id))
        })
        .ok_or_else(|| McpError::NoSuchItem(name.to_owned()))?;
    let description = stave_app::view::describe(design, found)
        .ok_or_else(|| McpError::NoSuchItem(name.to_owned()))?;
    let mut text = format!("{} ({})\n", description.name, description.kind);
    for row in &description.rows {
        let _ = writeln!(text, "  {}: {}", row.label, row.value);
    }
    let rows: Vec<Value> = description
        .rows
        .iter()
        .map(|row| json!({ "label": row.label, "value": row.value }))
        .collect();
    Ok(answer(
        text,
        json!({ "name": description.name, "kind": description.kind, "rows": rows }),
    ))
}

/// A heading of the language guide.
struct Heading<'g> {
    /// 1 for `#`, 2 for `##`, ...
    level: usize,
    /// The title as written.
    title: &'g str,
    /// Where its line starts in the guide.
    start: usize,
}

/// The guide's headings, in order, leaving out lines in code blocks.
fn headings() -> Vec<Heading<'static>> {
    let mut headings = Vec::new();
    let mut in_code = false;
    let mut start = 0;
    for line in GRAMMAR.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if trimmed.starts_with("```") {
            in_code = !in_code;
        } else if !in_code && trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let title = trimmed.get(level..).unwrap_or("").trim();
            if !title.is_empty() {
                headings.push(Heading {
                    level,
                    title,
                    start,
                });
            }
        }
        start = start.saturating_add(line.len());
    }
    headings
}

/// A title as compared: lower case, without backticks.
fn comparable(title: &str) -> String {
    title.replace('`', "").trim().to_lowercase()
}

/// The guide's sections, as `grammar` without a topic answers.
fn contents() -> CallToolResult {
    let headings = headings();
    let mut text = String::from("The sections of the .stave language guide:\n");
    let mut sections = Vec::new();
    for heading in headings.iter().filter(|h| (2..=3).contains(&h.level)) {
        let indent = "  ".repeat(heading.level.saturating_sub(1));
        let _ = writeln!(text, "{indent}{}", heading.title);
        sections.push(json!({ "title": heading.title, "level": heading.level }));
    }
    text.push_str("Call grammar with one of them as the topic to read it.");
    answer(text, json!({ "sections": sections }))
}

/// The section of the guide whose title is `topic` (or, failing that,
/// the first whose title holds it), through to the next heading at its
/// level or above.
fn section(topic: &str) -> Result<CallToolResult, McpError> {
    let headings = headings();
    let wanted = comparable(topic);
    let index = headings
        .iter()
        .position(|heading| comparable(heading.title) == wanted)
        .or_else(|| {
            (!wanted.is_empty())
                .then(|| {
                    headings
                        .iter()
                        .position(|heading| comparable(heading.title).contains(&wanted))
                })
                .flatten()
        })
        .ok_or_else(|| McpError::NoSuchTopic {
            topic: topic.to_owned(),
            sections: headings
                .iter()
                .filter(|heading| heading.level == 2)
                .map(|heading| heading.title)
                .collect::<Vec<_>>()
                .join(", "),
        })?;
    let heading = headings.get(index).ok_or_else(|| McpError::NoSuchTopic {
        topic: topic.to_owned(),
        sections: String::new(),
    })?;
    let end = headings
        .iter()
        .skip(index.saturating_add(1))
        .find(|next| next.level <= heading.level)
        .map_or(GRAMMAR.len(), |next| next.start);
    let mut text = GRAMMAR
        .get(heading.start..end)
        .unwrap_or("")
        .trim_end()
        .to_owned();
    if let Some(rules) = formal_rules(heading.title) {
        let _ = write!(
            text,
            "\n\nThe formal rules, from \"Syntactic grammar\":\n\n```\n{rules}\n```"
        );
    }
    Ok(answer(
        text.clone(),
        json!({ "topic": heading.title, "text": text }),
    ))
}

/// The group of rules in the guide's formal grammar (its EBNF block,
/// where groups start `(* ---------- Patches (...) ---------- *)`) whose
/// title starts with the section's title, if any: the exact syntax to go
/// with a section's prose.
fn formal_rules(title: &str) -> Option<&'static str> {
    const MARK: &str = "(* ----------";
    let wanted = comparable(title);
    let mut offset = 0;
    let mut group: Option<usize> = None;
    for line in GRAMMAR.split_inclusive('\n') {
        let trimmed = line.trim();
        let ends_group = trimmed.starts_with(MARK) || trimmed.starts_with("```");
        if ends_group && let Some(start) = group.take() {
            return GRAMMAR.get(start..offset).map(str::trim_end);
        }
        if let Some(rest) = trimmed.strip_prefix(MARK) {
            let name = rest
                .trim_end_matches("*)")
                .trim()
                .trim_end_matches('-')
                .trim();
            if !wanted.is_empty() && comparable(name).starts_with(&wanted) {
                group = Some(offset);
            }
        }
        offset = offset.saturating_add(line.len());
    }
    None
}

/// The words a standard declaration starts with.
const DECLARATIONS: [&str; 3] = ["kind", "recipe", "profile"];

/// The declaration of `name` in the standard modules, with its doc
/// comment.
fn lookup(name: &str) -> Result<CallToolResult, McpError> {
    let name = name.trim();
    for (module, file, text) in stave_modules::std_files() {
        let Some(found) = declaration(text, name) else {
            continue;
        };
        let words = format!(
            "From std:{module} ({file}). The modules joinery, hardware and tools are seen \
             everywhere; import any other with `import std:{module}`.\n\n{found}\n"
        );
        return Ok(answer(
            words,
            json!({ "name": name, "module": module, "file": file, "text": found }),
        ));
    }
    Err(McpError::NoSuchKind(name.to_owned()))
}

/// The text of the declaration of `name` in a standard file: its doc
/// comment lines, then the declaration through its closing brace.
fn declaration<'t>(text: &'t str, name: &str) -> Option<&'t str> {
    let mut offset = 0;
    let mut doc_start: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("///") {
            doc_start.get_or_insert(offset);
        } else if declares(trimmed, name) {
            let start = doc_start.unwrap_or(offset);
            let end = block_end(text, offset)?;
            return text.get(start..end);
        } else {
            doc_start = None;
        }
        offset = offset.saturating_add(line.len());
    }
    None
}

/// Whether a line starts the declaration of `name`: `kind name : joint {`.
fn declares(line: &str, name: &str) -> bool {
    let line = line.strip_prefix("pub ").unwrap_or(line);
    DECLARATIONS.iter().any(|word| {
        line.strip_prefix(word)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|rest| rest.trim_start().strip_prefix(name))
            .is_some_and(|rest| {
                rest.chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
            })
    })
}

/// Where the block that starts on the line at `start` ends: just after
/// its matching closing brace (strings and `//` comments skipped), or at
/// the end of the line when it opens no block.
fn block_end(text: &str, start: usize) -> Option<usize> {
    let rest = text.get(start..)?;
    let mut depth = 0_usize;
    let mut opened = false;
    let mut in_string = false;
    let mut in_comment = false;
    let mut escaped = false;
    let mut previous = '\0';
    for (index, c) in rest.char_indices() {
        if in_comment {
            if c == '\n' {
                in_comment = false;
                if !opened {
                    return Some(start.saturating_add(index));
                }
            }
        } else if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else {
            match c {
                '"' => in_string = true,
                '/' if previous == '/' => in_comment = true,
                '{' => {
                    depth = depth.saturating_add(1);
                    opened = true;
                }
                '}' => {
                    depth = depth.saturating_sub(1);
                    if opened && depth == 0 {
                        return Some(start.saturating_add(index).saturating_add(1));
                    }
                }
                '\n' if !opened => return Some(start.saturating_add(index)),
                _ => {}
            }
        }
        previous = c;
    }
    Some(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declaration_comes_with_its_doc_and_whole_block() {
        let text = "stave 1\n\n/// A thing.\n/// It has \"}\" in it.\nkind thing : joint {\n  cut a: x { depth: 1 } // }\n  label: \"{\"\n}\nkind thing_two : joint {\n}\n";
        assert_eq!(
            declaration(text, "thing"),
            Some(
                "/// A thing.\n/// It has \"}\" in it.\nkind thing : joint {\n  cut a: x { depth: 1 } // }\n  label: \"{\"\n}"
            )
        );
        assert_eq!(
            declaration(text, "thing_two"),
            Some("kind thing_two : joint {\n}")
        );
        assert_eq!(declaration(text, "thin"), None);
    }

    #[test]
    fn every_heading_is_found_outside_code() {
        let headings = headings();
        assert!(
            headings
                .iter()
                .any(|h| h.level == 2 && h.title == "Patches")
        );
        assert!(headings.iter().all(|h| !h.title.starts_with('!')));
    }
}
