//! Laying a file out the way `stave fmt` does, for the command line and the
//! editor's "Format" alike.

use stave_lang::diagnostics::SourceMapError;
use stave_lang::{FormatError, SourceMap, TextEdit};

/// What laying a file out gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Formatted {
    /// The file's new text.
    Changed(String),
    /// The file was already laid out.
    Unchanged,
}

/// Why a file was not laid out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatTextError {
    /// The text couldn't be read as a source file.
    #[error(transparent)]
    Source(#[from] SourceMapError),
    /// The file has syntax errors, or the layout would change its meaning.
    #[error(transparent)]
    Format(#[from] FormatError),
}

/// Lays out one file's text as `stave fmt` does. `path` names the file in
/// the diagnostics of a file that doesn't parse.
pub fn format_text(path: &str, text: &str) -> Result<Formatted, FormatTextError> {
    let mut sources = SourceMap::new();
    let file = sources.add(path, text)?;
    let formatted = stave_lang::format_file(file, text)?;
    Ok(if formatted == text {
        Formatted::Unchanged
    } else {
        Formatted::Changed(formatted)
    })
}

/// Lays out one file's text as `stave fmt` does, as the smallest edits to
/// it (last in the file first; none when it is already laid out), for an
/// editor to apply to its buffer so the cursor and undo history stay.
pub fn format_edits(path: &str, text: &str) -> Result<Vec<TextEdit>, FormatTextError> {
    Ok(match format_text(path, text)? {
        Formatted::Changed(formatted) => stave_lang::text_edits(text, &formatted),
        Formatted::Unchanged => Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_laid_out_file_is_unchanged() {
        let Ok(Formatted::Changed(once)) =
            format_text("box.stave", "stave   1\n\n\n\npart  a:s{\n}\n")
        else {
            panic!("the badly spaced file changes");
        };
        assert_eq!(format_text("box.stave", &once), Ok(Formatted::Unchanged));
    }

    #[test]
    fn layout_is_given_as_edits_to_the_text() {
        let text = "stave   1\n\npart  a:s{\n}\n";
        let edits = format_edits("box.stave", text).unwrap();
        assert!(!edits.is_empty());
        let Ok(Formatted::Changed(whole)) = format_text("box.stave", text) else {
            panic!("the badly spaced file changes");
        };
        assert_eq!(
            stave_lang::apply_text_edits(text, &edits).as_deref(),
            Some(whole.as_str())
        );
        assert_eq!(format_edits("box.stave", &whole), Ok(Vec::new()));
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        assert!(matches!(
            format_text("box.stave", "stave 1\npart {{{"),
            Err(FormatTextError::Format(FormatError::Syntax(_)))
        ));
    }
}
