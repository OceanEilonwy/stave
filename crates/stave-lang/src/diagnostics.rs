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
//!
//! A diagnostic's help may carry [`Fix`]es: patches a tool can apply as
//! they are, each printed on its own line after the help (grammar,
//! "Fixes"):
//!
//! ```text
//!    = help: use slot: 8mm, from the kind's list
//!    = fix: patch "Use slot: 8mm" { set corner.slot = 8mm }
//! ```

use std::fmt::{self, Write as _};

use crate::ast::{self, Span};
use crate::{FORMAT_VERSION, Parse, parse_patch_file};

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
    /// `nothing is the root: no part is placed with 'ground' and no region
    /// is made with 'box'`, or two roots (`'<a>' and '<b>' are both placed
    /// with 'ground'`), a root inside a component, or `box(...)` that isn't
    /// the whole value of a region
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
    /// `Stave could not work out <what>; this is a bug in Stave`
    Internal = "internal", layer 3;
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
    /// Warning: wood joined or held against its grain: end grain glued,
    /// glue across the grain, a tenon with the grain across it, or a solid
    /// part held fast too far apart across its grain.
    Grain = "grain", layer 4;
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
    /// The messages of the other problems with the same code at the same
    /// place that the checker merged into this one, in full and in their
    /// order (its note names the first three). Empty for a diagnostic
    /// that stands for one problem. An editor comparing diagnostics
    /// across a change compares these too: a new problem on a line that
    /// already had one is only here.
    pub merged: Vec<String>,
    /// A concrete suggestion, printed as `= help: ...`.
    pub help: Option<String>,
    /// Patches that make the problem go away, printed as `= fix: ...`
    /// after the help, the one the help recommends first. Empty when the
    /// help asks for a judgement.
    pub fixes: Vec<Fix>,
    /// Why it was reported, for the causes a tool treats apart from the
    /// rest; `None` for most diagnostics.
    pub cause: Option<Cause>,
}

/// A cause of a diagnostic that a tool treats apart from the rest
/// (grammar, "Kind values"). A diagnostic with a cause says it in one
/// plain sentence, its first note ([`Diagnostic::reason`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cause {
    /// A kind value that reached a joint or hardware declaration through
    /// a param doesn't take what the declaration hands it: a setting or a
    /// role it doesn't have, or several parts where it takes one. It can
    /// never be used there, whatever the sizes.
    KindSignature,
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
            merged: Vec::new(),
            help: None,
            fixes: Vec::new(),
            cause: None,
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

    /// Adds a fix after the ones already there.
    #[must_use]
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fixes.push(fix);
        self
    }

    /// Adds a note.
    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Records why it was reported, with the reason in one plain
    /// sentence, which becomes the first note.
    #[must_use]
    pub fn with_cause(mut self, cause: Cause, reason: impl Into<String>) -> Self {
        self.notes.insert(0, reason.into());
        self.cause = Some(cause);
        self
    }

    /// The plain one-sentence reason of a diagnostic with a cause: its
    /// first note.
    pub fn reason(&self) -> Option<&str> {
        self.cause?;
        self.notes.first().map(String::as_str)
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

/// A fix: a labelled patch that makes a diagnostic go away, which a tool
/// can apply as it is (grammar, "Fixes"). It is written on one line,
/// `patch "Use 18mm" { set door.overlay = 18mm }`, and always parses with
/// the patch parser: [`Fix::new`] and [`Fix::parse`] refuse anything else.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Fix {
    label: String,
    operations: String,
}

/// Why a [`Fix`] can't be made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FixError {
    /// The label is empty.
    #[error("a fix needs a label, such as \"Use 18mm\"")]
    EmptyLabel,
    /// The label doesn't start with a capital letter.
    #[error("a fix's label starts with a capital letter, found \"{0}\"")]
    LowercaseLabel(String),
    /// The label ends with a full stop.
    #[error("a fix's label has no full stop at the end, found \"{0}\"")]
    FullStop(String),
    /// The label holds a character a plain string can't.
    #[error(
        "a fix's label can't hold a quote, a backslash, a brace or a line break, found \"{0}\""
    )]
    LabelCharacter(String),
    /// The patch is written on more than one line.
    #[error("a fix is written on one line")]
    MultiLine,
    /// The patch has no operations.
    #[error("a fix needs at least one operation")]
    NoOperations,
    /// The text is not exactly one patch.
    #[error("a fix is exactly one patch: patch \"<label>\" {{ <operations> }}")]
    NotOnePatch,
    /// The patch doesn't parse.
    #[error("the fix's patch has {} syntax error(s)", .0.len())]
    Syntax(Vec<Diagnostic>),
}

impl Fix {
    /// A fix with a label (the text on its button: a short instruction
    /// with a capital letter and no full stop, "Use 18mm") and its
    /// operations, written on one line and separated by commas
    /// (`set door.overlay = 18mm, unset door.gap`).
    ///
    /// # Errors
    ///
    /// A label the grammar doesn't allow, or operations that are not one
    /// line of the patch grammar.
    pub fn new(label: impl Into<String>, operations: &str) -> Result<Self, FixError> {
        let label = label.into();
        check_label(&label)?;
        if operations.contains(['\n', '\r']) {
            return Err(FixError::MultiLine);
        }
        let text = patch_file_text(&format!("patch \"{label}\" {{ {operations} }}"));
        let parse = parse_patch_file(FileId::DETACHED, &text);
        let (_, operations) = one_patch(&parse, &text)?;
        Ok(Self { label, operations })
    }

    /// Reads a fix as it is printed after `= fix: `, or written on a
    /// broken design's `// fix:` line: `patch "Use 18mm" { ... }`.
    ///
    /// # Errors
    ///
    /// Text that is not exactly one patch on one line, with a label the
    /// grammar allows and at least one operation.
    pub fn parse(text: &str) -> Result<Self, FixError> {
        let text = text.trim();
        if text.contains(['\n', '\r']) {
            return Err(FixError::MultiLine);
        }
        let file = patch_file_text(text);
        let parse = parse_patch_file(FileId::DETACHED, &file);
        let (label, operations) = one_patch(&parse, &file)?;
        Self::new(label, &operations)
    }

    /// The label on the fix's button.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The operations, on one line, separated by commas.
    pub fn operations(&self) -> &str {
        &self.operations
    }

    /// The fix as a patch file, `stave 1` header included, ready for
    /// [`crate::parse_patch_file`] and [`crate::apply_patch`].
    pub fn patch_file(&self) -> String {
        patch_file_text(&self.to_string())
    }
}

impl fmt::Display for Fix {
    /// The patch on one line: `patch "<label>" { <operations> }`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "patch \"{}\" {{ {} }}", self.label, self.operations)
    }
}

fn patch_file_text(patch: &str) -> String {
    format!("stave {FORMAT_VERSION}\n\n{patch}\n")
}

fn check_label(label: &str) -> Result<(), FixError> {
    let Some(first) = label.chars().next() else {
        return Err(FixError::EmptyLabel);
    };
    if label.contains(['"', '\\', '{', '}', '\n', '\r']) {
        return Err(FixError::LabelCharacter(label.to_owned()));
    }
    if !first.is_uppercase() {
        return Err(FixError::LowercaseLabel(label.to_owned()));
    }
    if label.ends_with('.') {
        return Err(FixError::FullStop(label.to_owned()));
    }
    Ok(())
}

/// The label and the operations (each as written, joined by ", ") of a
/// patch file that holds exactly one patch.
fn one_patch(parse: &Parse<ast::PatchFile>, text: &str) -> Result<(String, String), FixError> {
    if parse.has_errors() {
        return Err(FixError::Syntax(
            parse
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.is_error())
                .cloned()
                .collect(),
        ));
    }
    let [patch] = parse
        .ast
        .as_ref()
        .map(|file| file.patches.as_slice())
        .unwrap_or_default()
    else {
        return Err(FixError::NotOnePatch);
    };
    if patch.ops.is_empty() {
        return Err(FixError::NoOperations);
    }
    let slice = |span: Span| {
        text.get(to_usize(span.start())..to_usize(span.end()))
            .unwrap_or("")
    };
    let mut label = String::new();
    for part in &patch.summary.parts {
        match part {
            ast::StringPart::Text { text, .. } => label.push_str(text),
            ast::StringPart::Interp { .. } => {
                return Err(FixError::LabelCharacter(
                    slice(patch.summary.span).to_owned(),
                ));
            }
        }
    }
    let operations: Vec<&str> = patch.ops.iter().map(|op| slice(op.span)).collect();
    Ok((label, operations.join(", ")))
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

/// Where a label lands once lines are known: its file, the line and
/// column it starts at, and how many characters [`render_diagnostic`]
/// underlines on that line. Viewers use it to show a diagnostic the way
/// the renderer prints it, without parsing the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement<'a> {
    /// The file the label is in.
    pub file: &'a SourceFile,
    /// Where the label starts.
    pub start: LineColumn,
    /// The number of characters underlined on the start line, at least one
    /// (to the end of the line for a span that runs onto later lines).
    pub width: usize,
}

impl Placement<'_> {
    /// The start line's text, without its line ending.
    pub fn line_text(&self) -> &str {
        self.file.line_text(self.start.line)
    }
}

/// Places a label in its file, or `None` when the file isn't in the map.
pub fn place_label<'a>(sources: &'a SourceMap, label: &Label) -> Option<Placement<'a>> {
    let file = sources.get(label.file)?;
    let start = file.line_column(label.span.start());
    let end = file.line_column(label.span.end());
    let line_chars = file.line_text(start.line).chars().count();
    let last_column = if end.line == start.line {
        end.column
    } else {
        line_chars.saturating_add(1)
    };
    Some(Placement {
        file,
        start,
        width: last_column.saturating_sub(start.column).max(1),
    })
}

/// Where a label lands, with the label.
struct Placed<'a> {
    label: &'a Label,
    file: &'a SourceFile,
    start: LineColumn,
    /// Number of characters to underline on the start line, at least one.
    width: usize,
}

fn place<'a>(sources: &'a SourceMap, label: &'a Label) -> Option<Placed<'a>> {
    let placement = place_label(sources, label)?;
    Some(Placed {
        label,
        file: placement.file,
        start: placement.start,
        width: placement.width,
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
    for fix in &diagnostic.fixes {
        let _ = writeln!(out, "{pad} = fix: {fix}");
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

    #[test]
    fn renders_each_fix_on_its_own_line_after_the_help() {
        let mut sources = SourceMap::new();
        let id = sources.add("a.stave", "let slot = 6mm\n").unwrap();
        let diagnostic = Diagnostic::error(
            Code::Require,
            "'thirds' fails: the slot is a third of the post's thickness",
            Label::new(id, Span::new(4, 8)),
        )
        .with_help("use slot: 8mm or slot: 9mm, from the kind's list")
        .with_fix(Fix::new("Use slot: 8mm", "set slot = 8mm").unwrap())
        .with_fix(Fix::new("Use slot: 9mm", "set slot = 9mm,   unset other").unwrap());
        assert_eq!(
            render_diagnostic(&sources, &diagnostic),
            "error[require]: 'thirds' fails: the slot is a third of the post's thickness\n\
             \x20 --> a.stave:1:5\n\
             \x20  |\n\
             \x201 | let slot = 6mm\n\
             \x20  |     ^^^^\n\
             \x20  = help: use slot: 8mm or slot: 9mm, from the kind's list\n\
             \x20  = fix: patch \"Use slot: 8mm\" { set slot = 8mm }\n\
             \x20  = fix: patch \"Use slot: 9mm\" { set slot = 9mm, unset other }\n"
        );
    }

    #[test]
    fn the_grammars_fix_lines_parse_and_print_back_the_same() {
        let mut seen = 0;
        for line in GRAMMAR.lines() {
            let Some(fix) = line.trim_start().strip_prefix("= fix: ") else {
                continue;
            };
            let parsed = Fix::parse(fix).unwrap_or_else(|error| panic!("{fix}: {error}"));
            assert_eq!(parsed.to_string(), fix);
            assert_eq!(Fix::parse(&parsed.to_string()), Ok(parsed.clone()));
            let file = parsed.patch_file();
            assert!(!crate::parse_patch_file(FileId::DETACHED, &file).has_errors());
            seen += 1;
        }
        assert!(seen >= 2, "the grammar's Fixes section lost its examples");
    }

    #[test]
    fn a_fix_is_one_labelled_patch_on_one_line() {
        let fix = Fix::new("Remove 'locked'", "set reveal = 1.5mm").unwrap();
        assert_eq!(fix.label(), "Remove 'locked'");
        assert_eq!(fix.operations(), "set reveal = 1.5mm");
        assert_eq!(
            fix.patch_file(),
            "stave 1\n\npatch \"Remove 'locked'\" { set reveal = 1.5mm }\n"
        );
        assert_eq!(Fix::new("", "set a = 1mm"), Err(FixError::EmptyLabel));
        assert!(matches!(
            Fix::new("use 18mm", "set a = 1mm"),
            Err(FixError::LowercaseLabel(_))
        ));
        assert!(matches!(
            Fix::new("Use 18mm.", "set a = 1mm"),
            Err(FixError::FullStop(_))
        ));
        assert!(matches!(
            Fix::new("Use {a}", "set a = 1mm"),
            Err(FixError::LabelCharacter(_))
        ));
        assert_eq!(
            Fix::new("Use 18mm", "set a = 1mm\nset b = 2mm"),
            Err(FixError::MultiLine)
        );
        assert_eq!(Fix::new("Use 18mm", ""), Err(FixError::NoOperations));
        assert!(matches!(
            Fix::new("Use 18mm", "set = 1mm"),
            Err(FixError::Syntax(_))
        ));
        assert!(matches!(
            Fix::new("Use 18mm", "set a = 1mm } patch \"B\" { set b = 2mm"),
            Err(FixError::NotOnePatch | FixError::Syntax(_))
        ));
        assert_eq!(
            Fix::parse("patch \"A\" { set a = 1mm }\npatch \"B\" { set b = 1mm }"),
            Err(FixError::MultiLine)
        );
    }
}
