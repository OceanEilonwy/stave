//! Quick fixes from the help lines of the fast tier's diagnostics
//! (grammar, "Tooling and editors": "for a syntax error, its `help:` line
//! as a text edit when it is mechanical"). A help line is mechanical when
//! it names exactly the text to write and where: `add "}" before line 12`,
//! `add ")" at the end of line 7`, `write ')' here`, `built-in functions are calls: write mid(base, up)`,
//! `give the module another name: import "./hw" as hw`. A help line that
//! asks for a judgement gives no fix.
//!
//! The checker's own fixes are patches, applied in the background
//! (`stave_app::editor_check`); these come from the loader and the
//! parser, so they are there on every keystroke.

use stave_lang::ast::Span;
use stave_lang::{Code, Diagnostic, FileId};

use crate::analysis::Analysis;

/// A text edit that does what a diagnostic's help line says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpFix {
    /// The diagnostic it fixes.
    pub diagnostic: Diagnostic,
    /// What the fix does, for its button: `Add ")"`.
    pub label: String,
    /// The text replaced; empty for an insertion.
    pub span: Span,
    /// What replaces it.
    pub text: String,
}

/// Closing punctuation a `write 'x' here` help line may insert.
const INSERTABLE: [&str; 6] = [")", "]", "}", ":", "=", ","];

impl Analysis {
    /// The fixes a file's fast-tier diagnostics' help lines give, in the
    /// order of the diagnostics.
    pub fn help_fixes(&self, file: FileId) -> Vec<HelpFix> {
        let Some(text) = self.text(file) else {
            return Vec::new();
        };
        self.diagnostics(file)
            .filter_map(|diagnostic| help_fix(diagnostic, file, text))
            .collect()
    }
}

/// The fix a diagnostic's help line gives, when it is mechanical.
fn help_fix(diagnostic: &Diagnostic, file: FileId, text: &str) -> Option<HelpFix> {
    let primary = diagnostic.primary()?;
    if primary.file != file {
        return None;
    }
    let help = diagnostic.help.as_deref()?;
    let span = primary.span;
    let fix = |label: String, span: Span, text: String| HelpFix {
        diagnostic: diagnostic.clone(),
        label,
        span,
        text,
    };
    if diagnostic.code == Code::MissingBrace {
        let insert = if help == "add \"}\" at the end of the file" {
            let end = u32::try_from(text.len()).ok()?;
            let text = if text.ends_with('\n') || text.is_empty() {
                "}\n"
            } else {
                "\n}\n"
            };
            (end, text)
        } else {
            let line = help.strip_prefix("add \"}\" before line ")?;
            let line: usize = line.trim().parse().ok()?;
            (line_start(text, line)?, "}\n")
        };
        return Some(fix(
            "Add \"}\"".to_owned(),
            Span::new(insert.0, insert.0),
            insert.1.to_owned(),
        ));
    }
    if diagnostic.code == Code::UnclosedBracket {
        // `add ")" at the end of line 7`: before a comment on that line.
        let (closer, line) = help
            .strip_prefix("add \"")?
            .split_once("\" at the end of line ")?;
        if !INSERTABLE.contains(&closer) {
            return None;
        }
        let at = line_code_end(text, line.trim().parse().ok()?)?;
        return Some(fix(
            format!("Add \"{closer}\""),
            Span::new(at, at),
            closer.to_owned(),
        ));
    }
    if diagnostic.code == Code::Syntax {
        if let Some((_, call)) = help.split_once(" are calls: write ") {
            return call_fix(call, span, text)
                .map(|(span, call)| fix(format!("Write {call}"), span, call));
        }
        let written = help
            .strip_prefix("write '")
            .and_then(|rest| rest.strip_suffix("' here"))?;
        if !INSERTABLE.contains(&written) {
            return None;
        }
        return Some(fix(
            format!("Add \"{written}\""),
            Span::new(span.start(), span.start()),
            written.to_owned(),
        ));
    }
    if diagnostic.code == Code::ReservedWord || diagnostic.code == Code::DuplicateName {
        // `import "./hw"` with no `as` name: the help's suggestion adds one.
        let (_, suggestion) = help.rsplit_once(": import \"")?;
        let (_, alias) = suggestion.rsplit_once("\" as ")?;
        let alias = alias.trim();
        let written = covered(text, span)?;
        let is_word = !alias.is_empty() && alias.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !written.starts_with('"') || !is_word {
            return None;
        }
        return Some(fix(
            format!("Add \"as {alias}\""),
            Span::new(span.end(), span.end()),
            format!(" as {alias}"),
        ));
    }
    None
}

/// For `built-in functions are calls: write mid(base, up)` at `base`: the
/// span from the head (`mid`) to the end of its arguments as written, and
/// the call. `None` when the help could not name the arguments (`...`) or
/// the text is not as the help read it.
fn call_fix(call: &str, at: Span, text: &str) -> Option<(Span, String)> {
    let (head, rest) = call.split_once('(')?;
    let arguments = rest.strip_suffix(')')?;
    if arguments == "..." || head.is_empty() {
        return None;
    }
    let start = usize::try_from(at.start()).ok()?;
    let before = text.get(..start)?;
    let trimmed = before.trim_end_matches([' ', '\t']);
    let head_start = trimmed.strip_suffix(head).map(str::len)?;
    let end = start.checked_add(arguments.len())?;
    if text.get(start..end)? != arguments {
        return None;
    }
    Some((
        Span::new(u32::try_from(head_start).ok()?, u32::try_from(end).ok()?),
        call.to_owned(),
    ))
}

/// The byte offset of a 1-based line's start.
fn line_start(text: &str, line: usize) -> Option<u32> {
    let mut offset = 0usize;
    for _ in 1..line {
        offset = offset
            .checked_add(text.get(offset..)?.find('\n')?)?
            .checked_add(1)?;
    }
    u32::try_from(offset).ok()
}

/// The byte offset just after the last code on a 1-based line: before a
/// `//` comment that ends it and the spaces before that.
fn line_code_end(text: &str, line: usize) -> Option<u32> {
    let start = usize::try_from(line_start(text, line)?).ok()?;
    let rest = text.get(start..)?;
    let line_text = rest.split('\n').next()?;
    let mut in_string = false;
    let mut code_end = line_text.len();
    let mut previous = None;
    for (at, c) in line_text.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '/' if !in_string && previous == Some('/') => {
                code_end = at.saturating_sub(1);
                break;
            }
            _ => {}
        }
        previous = Some(c);
    }
    let code = line_text.get(..code_end)?.trim_end();
    u32::try_from(start.checked_add(code.len())?).ok()
}

/// The text a span covers.
fn covered(text: &str, span: Span) -> Option<&str> {
    text.get(usize::try_from(span.start()).ok()?..usize::try_from(span.end()).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::Label;

    fn fixed(text: &str, fix: &HelpFix) -> String {
        let mut out = text.to_owned();
        out.replace_range(
            fix.span.start() as usize..fix.span.end() as usize,
            &fix.text,
        );
        out
    }

    fn at(text: &str, needle: &str) -> Span {
        let start = u32::try_from(text.find(needle).unwrap()).unwrap();
        Span::new(start, start + u32::try_from(needle.len()).unwrap())
    }

    fn file() -> FileId {
        stave_lang::SourceMap::new().add("/p/a.stave", "").unwrap()
    }

    #[test]
    fn a_missing_closing_bracket_is_added() {
        let text = "let x = mid(base, up\n";
        let diagnostic = Diagnostic::error(
            Code::Syntax,
            "expected ')', found end of line",
            Label::new(file(), at(text, "\n")),
        )
        .with_help("write ')' here");
        let fix = help_fix(&diagnostic, file(), text).unwrap();
        assert_eq!(fix.label, "Add \")\"");
        assert_eq!(fixed(text, &fix), "let x = mid(base, up)\n");
    }

    #[test]
    fn a_missing_brace_is_added_before_the_next_declaration() {
        let text = "part a : oak {\n  length: 1mm\npart b : oak {\n}\n";
        let diagnostic = Diagnostic::error(
            Code::MissingBrace,
            "part a is missing its closing \"}\"",
            Label::new(file(), at(text, "{")),
        )
        .with_help("add \"}\" before line 3");
        let fix = help_fix(&diagnostic, file(), text).unwrap();
        assert_eq!(
            fixed(text, &fix),
            "part a : oak {\n  length: 1mm\n}\npart b : oak {\n}\n"
        );
        let at_end = Diagnostic::error(
            Code::MissingBrace,
            "part a is missing its closing \"}\"",
            Label::new(file(), at(text, "{")),
        )
        .with_help("add \"}\" at the end of the file");
        let fix = help_fix(&at_end, file(), "part a : oak {").unwrap();
        assert_eq!(fixed("part a : oak {", &fix), "part a : oak {\n}\n");
    }

    #[test]
    fn a_call_without_parentheses_gets_them() {
        let text = "  flush base.top\n";
        let diagnostic = Diagnostic::error(
            Code::Syntax,
            "expected end of line, found 'base'",
            Label::new(file(), at(text, "base")),
        )
        .with_help("placement terms are calls: write flush(base.top)");
        let fix = help_fix(&diagnostic, file(), text).unwrap();
        assert_eq!(fixed(text, &fix), "  flush(base.top)\n");
    }

    #[test]
    fn an_import_named_with_a_reserved_word_gets_an_as_name() {
        let text = "import \"./part\"\n";
        let diagnostic = Diagnostic::error(
            Code::ReservedWord,
            "'part' is a reserved word and can't be used as a name",
            Label::new(file(), at(text, "\"./part\"")),
        )
        .with_help("give the module another name: import \"./part\" as part_module");
        let fix = help_fix(&diagnostic, file(), text).unwrap();
        assert_eq!(fixed(text, &fix), "import \"./part\" as part_module\n");
    }

    #[test]
    fn an_unclosed_bracket_is_closed_before_the_lines_comment() {
        let text = "let x = mid(base, up  // the middle\nlet y = 1\n";
        let diagnostic = Diagnostic::error(
            Code::UnclosedBracket,
            "this \"(\" is never closed",
            Label::new(file(), at(text, "(")),
        )
        .with_help("add \")\" at the end of line 1");
        let fix = help_fix(&diagnostic, file(), text).unwrap();
        assert_eq!(
            fixed(text, &fix),
            "let x = mid(base, up)  // the middle\nlet y = 1\n"
        );
    }

    #[test]
    fn a_help_line_asking_for_a_judgement_gives_no_fix() {
        let text = "let = 3\n";
        let diagnostic = Diagnostic::error(
            Code::Syntax,
            "expected a name, found '='",
            Label::new(file(), at(text, "=")),
        )
        .with_help("write a name: letters, digits and _, starting with a letter or _");
        assert_eq!(help_fix(&diagnostic, file(), text), None);
    }
}
