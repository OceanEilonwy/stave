//! Laying a file out the way `stave fmt` does, for the command line and the
//! editor's "Format" alike.

use stave_lang::diagnostics::SourceMapError;
use stave_lang::{FormatError, SourceMap};

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
    fn a_file_that_does_not_parse_is_left_alone() {
        assert!(matches!(
            format_text("box.stave", "stave 1\npart {{{"),
            Err(FormatTextError::Format(FormatError::Syntax(_)))
        ));
    }
}
