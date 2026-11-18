//! Diagnostics shared by every stage: source files, codes, labels and a
//! compiler-style renderer.
//!
//! A diagnostic points into source text with byte-offset [`Span`]s. Lines and
//! columns are worked out only when a diagnostic is rendered (grammar,
//! "Tooling and editors": positions are byte offsets, computed into lines and
//! columns at the edge).
//!
//! ```text
//! error[syntax]: expected ':', found 'shelf'
//!   --> project.stave:20:21
//!    |
//! 20 | require wide_enough shelf.width >= 250mm "the shelf must hold a row of books"
//!    |                     ^^^^^
//!    = help: write ':' between the name and the condition
//! ```

use std::fmt::{self, Write as _};

use crate::ast::Span;

/// Identifies a file in a [`SourceMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(u32);

impl FileId {
    /// An id for text that is not in any map, for internal lexing whose
    /// diagnostics are thrown away.
    pub(crate) const DETACHED: FileId = FileId(u32::MAX);

    /// The file's index in its [`SourceMap`], in the order files were added.
    pub fn index(self) -> u32 {
        self.0
    }
}

/// Why a file can't be added to a [`SourceMap`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceMapError {
    /// The map already holds `u32::MAX` files.
    #[error("too many source files")]
    TooManyFiles,
    /// Spans are 32-bit byte offsets, so a file is at most 4 GiB.
    #[error("'{path}' is larger than 4 GiB")]
    FileTooLarge {
        /// The file's path.
        path: String,
    },
}

/// One source file: its display path, its text and where its lines start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    path: String,
    text: String,
    /// Byte offset of the first character of every line; always starts at 0.
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// The path shown in diagnostics.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The file's text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The 1-based line and column (in characters) of a byte offset. An
    /// offset past the end is clamped to the end of the file.
    pub fn line_column(&self, offset: u32) -> LineColumn {
        let offset = offset.min(u32::try_from(self.text.len()).unwrap_or(u32::MAX));
        let line_index = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        let before = self
            .text
            .get(to_usize(line_start)..to_usize(offset))
            .unwrap_or("");
        LineColumn {
            line: line_index.saturating_add(1),
            column: before.chars().count().saturating_add(1),
        }
    }

    /// The text of a 1-based line, without its line ending.
    pub fn line_text(&self, line: usize) -> &str {
        let Some(&start) = line.checked_sub(1).and_then(|i| self.line_starts.get(i)) else {
            return "";
        };
        let end = self
            .line_starts
            .get(line)
            .map_or(self.text.len(), |&next| to_usize(next));
        self.text
            .get(to_usize(start)..end)
            .unwrap_or("")
            .trim_end_matches(['\n', '\r'])
    }

    /// The number of lines in the file.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

/// A 1-based line and column. Columns count characters, not bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineColumn {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
}

/// Every source file of a run, by [`FileId`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file and returns its id.
    pub fn add(
        &mut self,
        path: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<FileId, SourceMapError> {
        let path = path.into();
        let text = text.into();
        if u32::try_from(text.len()).is_err() {
            return Err(SourceMapError::FileTooLarge { path });
        }
        let id = FileId(u32::try_from(self.files.len()).map_err(|_| SourceMapError::TooManyFiles)?);
        let mut line_starts = vec![0];
        line_starts.extend(
            text.match_indices('\n')
                .filter_map(|(offset, _)| u32::try_from(offset + 1).ok()),
        );
        self.files.push(SourceFile {
            path,
            text,
            line_starts,
        });
        Ok(id)
    }

    /// The file with this id.
    pub fn get(&self, file: FileId) -> Option<&SourceFile> {
        self.files.get(to_usize(file.0))
    }

    /// Every file, in the order they were added.
    pub fn files(&self) -> impl Iterator<Item = (FileId, &SourceFile)> {
        (0u32..)
            .zip(&self.files)
            .map(|(id, file)| (FileId(id), file))
    }
}

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The design can't be built until this is fixed.
    Error,
    /// The design builds, but something deserves a look.
    Warning,
    /// Information only.
    Note,
}

impl Severity {
    /// The word printed before the code: `error`, `warning`, `note`.
    pub fn word(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.word())
    }
}

macro_rules! codes {
    ($( $(#[doc = $doc:literal])* $variant:ident = $text:literal, layer $layer:literal; )*) => {
        /// A diagnostic code from the grammar's "Diagnostics" table. Codes are
        /// fixed because tests and AI tools match on them.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Code {
            $( $(#[doc = $doc])* $variant ),*
        }

        impl Code {
            /// Every code, in the order the grammar lists them.
            pub const ALL: &'static [Code] = &[$( Code::$variant ),*];

            /// The code as written in `error[code]`.
            pub fn as_str(self) -> &'static str {
                match self {
                    $( Code::$variant => $text ),*
                }
            }

            /// The checking layer that reports this code (grammar, "Checks and
            /// require"); 1 is syntax. For a code reported by two layers, the
            /// first.
            pub fn layer(self) -> u8 {
                match self {
                    $( Code::$variant => $layer ),*
                }
            }
        }
    };
}

codes! {
    /// `expected <what>, found <token>`
    Syntax = "syntax", layer 1;
    /// `'<name>' has no value`
    MissingValue = "missing-value", layer 1;
    /// `'<word>' is a reserved word and can't be used as a name`
    ReservedWord = "reserved-word", layer 1;
    /// `unknown unit '<unit>' (expected one of mm, cm, m, in, ft, deg)`
    UnknownUnit = "unknown-unit", layer 1;
    /// `this "<bracket>" is never closed`
    UnclosedBracket = "unclosed-bracket", layer 1;
    /// `<the body of part 'side_l'> is missing its closing "}"`
    MissingBrace = "missing-brace", layer 1;
    /// `unknown name '<name>'`
    UnknownName = "unknown-name", layer 2;
    /// `'<name>' is declared twice`
    DuplicateName = "duplicate-name", layer 2;
    /// `'<name>' could be <this> or <that> here`
    AmbiguousName = "ambiguous-name", layer 2;
    /// `can't <add> <a length> and <an angle>`
    UnitMismatch = "unit-mismatch", layer 2;
    /// `'<field>' needs <a part>, found <a region>`
    TypeMismatch = "type-mismatch", layer 2;
    /// `<a part> has no field '<name>'`
    UnknownField = "unknown-field", layer 2;
    /// `<what> is missing '<name>'`
    MissingField = "missing-field", layer 2;
    /// `'<param>' must be within <domain>, found <value>`
    OutOfRange = "out-of-range", layer 2;
    /// `a library can't contain a <part> at the top level`
    NotInLibrary = "not-in-library", layer 2;
    /// `modules import each other: <a> -> <b> -> <a>`
    ImportCycle = "import-cycle", layer 2;
    /// `can't find module '<source>'`
    UnknownModule = "unknown-module", layer 2;
    /// `<what> is not supported yet`
    NotSupported = "not-supported", layer 2;
    /// Warning: `this cut has no name, so messages will call it "a <slot>"`
    UnnamedCut = "unnamed-cut", layer 2;
    /// `no part is placed with 'ground'`
    Ground = "ground", layer 3;
    /// `<item>: position along '<axis>' is not set`
    AxisNotSet = "axis-not-set", layer 3;
    /// `<item>: position along '<axis>' is set twice`
    AxisSetTwice = "axis-set-twice", layer 3;
    /// `<a> and <b> are placed against each other`
    PlacementCycle = "placement-cycle", layer 3;
    /// `<part>: nothing decides its <length>`
    FitUndecided = "fit-undecided", layer 3;
    /// `<item> has no counterpart on the other side of the mirror`
    Mirror = "mirror", layer 3;
    /// `<kind>'s mechanism has <n> freedoms; it needs exactly one`
    Mechanism = "mechanism", layer 3;
    /// `<joint>: <part> meets none of the parts it should join`
    NoContact = "no-contact", layer 4;
    /// `<a> and <b> overlap`
    Overlap = "overlap", layer 4;
    /// `<hardware>: the <member> would sit <offset> off its place on the <member>; it adjusts <d>`
    Adjust = "adjust", layer 4;
    /// `<joint>: its <hole> in <part> breaks out of <part>'s <front edge>`,
    /// or `... runs into <other>'s <dado>`
    Wall = "wall", layer 4;
    /// `<joint>: its <tenon> on <part> is <30mm> thick, but <part> is only <19mm> thick`
    CutSize = "cut-size", layer 4;
    /// `<a> hits <b> when <assembly> is <open>`
    Motion = "motion", layer 5;
    /// `'<name>' fails: <message>`
    Require = "require", layer 6;
    /// `'<name>' does not hold for every allowed value: <message>`
    RequireRange = "require-range", layer 6;
    /// `<what> stops working when <param> <is below> <value>`
    Range = "range", layer 6;
    /// `<part>: there is no valid order for its cuts`
    Order = "order", layer 7;
    /// Warning: a local kind or recipe hides one from the prelude.
    ShadowsPrelude = "shadows-prelude", layer 2;
    /// Warning: no recipe can make a cut with the project's tools.
    NoRecipe = "no-recipe", layer 7;
    /// Warning: a hardware model does not match its envelope.
    ModelMismatch = "model-mismatch", layer 4;
    /// Warning: a part that no joint or hardware joins to the ground part.
    LoosePart = "loose-part", layer 4;
}

impl Code {
    /// The code for its text, if it is one.
    pub fn from_str_code(text: &str) -> Option<Code> {
        Code::ALL.iter().copied().find(|code| code.as_str() == text)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A span of source text with a short message. The first label of a
/// diagnostic is its primary location.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    /// The file the span is in.
    pub file: FileId,
    /// The text the label points at.
    pub span: Span,
    /// Shown next to the underline; may be empty.
    pub message: String,
}

impl Label {
    /// A label without a message.
    pub fn new(file: FileId, span: Span) -> Self {
        Self {
            file,
            span,
            message: String::new(),
        }
    }

    /// A label with a message.
    pub fn with_message(file: FileId, span: Span, message: impl Into<String>) -> Self {
        Self {
            file,
            span,
            message: message.into(),
        }
    }
}

/// A problem found in the source, compiler-style.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// The fixed code, e.g. `syntax`.
    pub code: Code,
    /// The message, in the shape the grammar fixes for the code.
    pub message: String,
    /// Where it is; the first label is the primary location.
    pub labels: Vec<Label>,
    /// Extra facts, printed as `= note: ...`.
    pub notes: Vec<String>,
    /// A concrete suggestion, printed as `= help: ...`.
    pub help: Option<String>,
}

impl Diagnostic {
    /// An error with a primary location.
    pub fn error(code: Code, message: impl Into<String>, primary: Label) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            labels: vec![primary],
            notes: Vec::new(),
            help: None,
        }
    }

    /// A warning with a primary location.
    pub fn warning(code: Code, message: impl Into<String>, primary: Label) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message, primary)
        }
    }

    /// Adds a help line.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Adds a note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Adds a secondary label.
    #[must_use]
    pub fn with_label(mut self, label: Label) -> Self {
        self.labels.push(label);
        self
    }

    /// The primary location, if there is one.
    pub fn primary(&self) -> Option<&Label> {
        self.labels.first()
    }

    /// `true` for errors.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Renders diagnostics compiler-style, one after another, each followed by a
/// blank line. Lines and columns are computed here from byte offsets.
pub fn render_diagnostics(sources: &SourceMap, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics {
        render_diagnostic_into(&mut out, sources, diagnostic);
        out.push('\n');
    }
    out
}

/// Renders one diagnostic, without a trailing blank line.
pub fn render_diagnostic(sources: &SourceMap, diagnostic: &Diagnostic) -> String {
    let mut out = String::new();
    render_diagnostic_into(&mut out, sources, diagnostic);
    out
}

/// Where a label lands once lines are known.
struct Placed<'a> {
    label: &'a Label,
    file: &'a SourceFile,
    start: LineColumn,
    /// Number of characters to underline on the start line, at least one.
    width: usize,
}

fn place<'a>(sources: &'a SourceMap, label: &'a Label) -> Option<Placed<'a>> {
    let file = sources.get(label.file)?;
    let start = file.line_column(label.span.start());
    let end = file.line_column(label.span.end());
    let line_chars = file.line_text(start.line).chars().count();
    let last_column = if end.line == start.line {
        end.column
    } else {
        line_chars.saturating_add(1)
    };
    Some(Placed {
        label,
        file,
        start,
        width: last_column.saturating_sub(start.column).max(1),
    })
}

fn render_diagnostic_into(out: &mut String, sources: &SourceMap, diagnostic: &Diagnostic) {
    // Writing into a String can't fail.
    let _ = write!(
        out,
        "{}[{}]: {}",
        diagnostic.severity, diagnostic.code, diagnostic.message
    );
    out.push('\n');
    let placed: Vec<Placed<'_>> = diagnostic
        .labels
        .iter()
        .filter_map(|label| place(sources, label))
        .collect();
    let widest_line = placed.iter().map(|p| p.start.line).max().unwrap_or(0);
    let gutter = widest_line.to_string().len().max(2);
    let pad = " ".repeat(gutter);

    for (index, placed) in placed.iter().enumerate() {
        let arrow = if index == 0 { "-->" } else { ":::" };
        let _ = writeln!(
            out,
            "{pad}{arrow} {}:{}:{}",
            placed.file.path(),
            placed.start.line,
            placed.start.column
        );
        let source_line = placed.file.line_text(placed.start.line).replace('\t', " ");
        let _ = writeln!(out, "{pad} |");
        let _ = writeln!(out, "{:>gutter$} | {source_line}", placed.start.line);
        let marker = if index == 0 { "^" } else { "-" };
        let mut underline = format!(
            "{}{}",
            " ".repeat(placed.start.column.saturating_sub(1)),
            marker.repeat(placed.width)
        );
        if !placed.label.message.is_empty() {
            underline.push(' ');
            underline.push_str(&placed.label.message);
        }
        let _ = writeln!(out, "{pad} | {underline}");
    }
    for note in &diagnostic.notes {
        let _ = writeln!(out, "{pad} = note: {note}");
    }
    if let Some(help) = &diagnostic.help {
        let _ = writeln!(out, "{pad} = help: {help}");
    }
}

fn to_usize(offset: u32) -> usize {
    usize::try_from(offset).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAMMAR: &str = include_str!("../../../docs/grammar.md");

    #[test]
    fn codes_match_the_grammar_table() {
        let start = "| Layer | Code | Message |\n|---|---|---|\n";
        let from = GRAMMAR.find(start).expect("diagnostics table") + start.len();
        let rest = &GRAMMAR[from..];
        let table = &rest[..rest.find("\n\n").expect("end of table")];
        let mut listed = Vec::new();
        for row in table.lines() {
            let cells: Vec<&str> = row.split('|').map(str::trim).collect();
            let code = cells[2].trim_matches('`');
            let layer: u8 = cells[1]
                .split(',')
                .next()
                .and_then(|layer| layer.trim().parse().ok())
                .expect("layer");
            let parsed = Code::from_str_code(code).unwrap_or_else(|| panic!("{code}"));
            assert_eq!(parsed.layer(), layer, "{code}");
            listed.push(code);
        }
        let warnings_start = "Warnings and notes: ";
        let from = GRAMMAR.find(warnings_start).expect("warnings") + warnings_start.len();
        let rest = &GRAMMAR[from..];
        for word in rest[..rest.find(".\n").expect("end")].split(',') {
            let code = word.trim().trim_matches('`');
            assert!(Code::from_str_code(code).is_some(), "{code}");
            if !listed.contains(&code) {
                listed.push(code);
            }
        }
        let all: Vec<&str> = Code::ALL.iter().map(|code| code.as_str()).collect();
        assert_eq!(all, listed);
    }

    #[test]
    fn lines_and_columns_count_characters() {
        let mut sources = SourceMap::new();
        let id = sources.add("a.stave", "ab\nçd\n\nlast").unwrap();
        let file = sources.get(id).unwrap();
        assert_eq!(file.line_column(0), LineColumn { line: 1, column: 1 });
        assert_eq!(file.line_column(2), LineColumn { line: 1, column: 3 });
        assert_eq!(file.line_column(3), LineColumn { line: 2, column: 1 });
        // 'ç' is two bytes but one column.
        assert_eq!(file.line_column(5), LineColumn { line: 2, column: 2 });
        assert_eq!(file.line_column(7), LineColumn { line: 3, column: 1 });
        assert_eq!(file.line_column(8), LineColumn { line: 4, column: 1 });
        assert_eq!(file.line_column(99), LineColumn { line: 4, column: 5 });
        assert_eq!(file.line_text(2), "çd");
        assert_eq!(file.line_text(4), "last");
        assert_eq!(file.line_text(5), "");
        assert_eq!(file.line_count(), 4);
        assert_eq!(sources.files().count(), 1);
    }

    #[test]
    fn renders_compiler_style() {
        let mut sources = SourceMap::new();
        let id = sources
            .add("box.stave", "part a : s {\n  length: 3inch\n}\n")
            .unwrap();
        let diagnostic = Diagnostic::error(
            Code::UnknownUnit,
            "unknown unit 'inch' (expected one of mm, cm, m, in, ft, deg)",
            Label::new(id, Span::new(24, 28)),
        )
        .with_note("units are written right after the number")
        .with_help("write 3in");
        assert_eq!(
            render_diagnostic(&sources, &diagnostic),
            "error[unknown-unit]: unknown unit 'inch' (expected one of mm, cm, m, in, ft, deg)\n\
             \x20 --> box.stave:2:12\n\
             \x20  |\n\
             \x202 |   length: 3inch\n\
             \x20  |            ^^^^\n\
             \x20  = note: units are written right after the number\n\
             \x20  = help: write 3in\n"
        );
    }
}
