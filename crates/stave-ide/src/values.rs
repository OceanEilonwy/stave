//! Worked-out values shown next to their source (grammar, "Tooling and
//! editors": hover and inlay hints): `length: fit` shows `= 844mm`, a
//! `between(base, top)` its value, every size its cut length.
//!
//! Values need the design solved, which is the slow tier's work, so they
//! are worked out in the background (`stave_app::editor_check`) and handed
//! here as [`ValueNote`]s. A note remembers the text it was worked out
//! for; while the user types, a note is shown only where the text it
//! covers is still the same at the same place, so a stale value is never
//! put next to text it was not worked out from. The next check replaces
//! them all.

use std::collections::BTreeMap;

use stave_lang::FileId;
use stave_lang::ast::Span;
use stave_lang::placement::PartAxis;

use crate::analysis::{Analysis, FilePosition};
use crate::document::Document;

/// What a value is the value of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ValueKind {
    /// A param's value.
    Param,
    /// A `let`'s value.
    Let,
    /// A part's size along one of its axes.
    Size(PartAxis),
    /// A `between(a, b)`.
    Between,
}

impl ValueKind {
    /// What it is, in words: "param", "length", "between".
    pub fn word(self) -> &'static str {
        match self {
            ValueKind::Param => "param",
            ValueKind::Let => "let",
            ValueKind::Size(PartAxis::Length) => "length",
            ValueKind::Size(PartAxis::Width) => "width",
            ValueKind::Size(PartAxis::Thickness) => "thickness",
            ValueKind::Between => "between",
        }
    }
}

/// One worked-out value, at the span of the text it is the value of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueNote {
    /// The document it is written in.
    pub document: Document,
    /// What it is shown next to: a param's default, a `let`'s value, a
    /// size's `fit` or expression, a whole `between(...)`.
    pub span: Span,
    /// The text `span` covered when the value was worked out.
    pub covers: String,
    /// For a param or `let`, its name and where it is: hovering the name
    /// shows the value too.
    pub name: Option<(Span, String)>,
    /// What it is the value of.
    pub kind: ValueKind,
    /// `true` when the text is a plain literal (`18mm`): the value is what
    /// is written, so no inlay hint repeats it.
    pub literal: bool,
    /// The value in the project's units: `844mm`, `844mm, cut 868mm`,
    /// `400mm, 450mm` (one per instance when they differ).
    pub text: String,
}

/// An inlay hint: a value shown after the text it is the value of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlayHint {
    /// The byte offset it is shown at: just after the text.
    pub offset: u32,
    /// What is shown: `= 844mm`.
    pub label: String,
    /// What the value is of.
    pub kind: ValueKind,
}

/// The worked-out values of one project, by document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkedValues {
    notes: BTreeMap<Document, Vec<ValueNote>>,
}

impl WorkedValues {
    /// Values from notes, in any order: each document's are kept sorted
    /// by span.
    pub fn new(notes: impl IntoIterator<Item = ValueNote>) -> WorkedValues {
        let mut by_document: BTreeMap<Document, Vec<ValueNote>> = BTreeMap::new();
        for note in notes {
            by_document
                .entry(note.document.clone())
                .or_default()
                .push(note);
        }
        for notes in by_document.values_mut() {
            notes.sort_by_key(|note| (note.span.start(), note.span.end(), note.kind));
        }
        WorkedValues { notes: by_document }
    }

    /// `true` when there are no values.
    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    /// Every note of a document, as worked out (stale ones included).
    pub fn notes(&self, document: &Document) -> &[ValueNote] {
        self.notes.get(document).map_or(&[], Vec::as_slice)
    }

    /// The notes of a loaded file whose text is still what they were
    /// worked out for.
    pub fn current<'a>(
        &'a self,
        analysis: &'a Analysis,
        file: FileId,
    ) -> impl Iterator<Item = &'a ValueNote> + 'a {
        let text = analysis.text(file).unwrap_or_default();
        let notes = analysis
            .document(file)
            .map_or(&[][..], |document| self.notes(&document));
        notes
            .iter()
            .filter(move |note| covered(text, note.span) == Some(note.covers.as_str()))
    }

    /// The inlay hints of a file, in order: every value that is not what
    /// is written, after the text it is the value of.
    pub fn inlay_hints(&self, analysis: &Analysis, file: FileId) -> Vec<InlayHint> {
        let mut hints: Vec<InlayHint> = self
            .current(analysis, file)
            .filter(|note| !note.literal)
            .map(|note| InlayHint {
                offset: note.span.end(),
                label: format!("= {}", note.text),
                kind: note.kind,
            })
            .collect();
        hints.sort_by_key(|hint| hint.offset);
        hints.dedup_by(|later, earlier| later.offset == earlier.offset);
        hints
    }

    /// The value shown for a position: the innermost value whose text
    /// holds it, or whose param or `let` the name there declares or names.
    pub fn at<'a>(
        &'a self,
        analysis: &'a Analysis,
        position: FilePosition,
    ) -> Option<&'a ValueNote> {
        let inside = |span: Span| span.start() <= position.offset && position.offset <= span.end();
        let own = self
            .current(analysis, position.file)
            .filter(|note| inside(note.span) || note.name.as_ref().is_some_and(|n| inside(n.0)))
            .min_by_key(|note| note.span.end().saturating_sub(note.span.start()));
        if own.is_some() {
            return own;
        }
        // A name of a param or `let` declared elsewhere.
        let declaration = analysis.definition(position)?;
        let text = analysis.text(declaration.file).unwrap_or_default();
        self.notes(&declaration.document).iter().find(|note| {
            note.name.as_ref().is_some_and(|(span, name)| {
                *span == declaration.span && covered(text, *span) == Some(name.as_str())
            })
        })
    }
}

/// The text a span covers, if it is inside the text.
fn covered(text: &str, span: Span) -> Option<&str> {
    let start = usize::try_from(span.start()).ok()?;
    let end = usize::try_from(span.end()).ok()?;
    text.get(start..end)
}
