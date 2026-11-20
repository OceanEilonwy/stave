//! Stave's answers as protocol values: diagnostics, completions,
//! signature help, hovers, code lenses, inlay hints, document symbols,
//! folding ranges and text edits.

use lsp_types::{
    CodeLens, Command, CompletionItem, CompletionItemKind, CompletionTextEdit,
    DiagnosticRelatedInformation, DiagnosticSeverity, DocumentSymbol, Documentation, FoldingRange,
    FoldingRangeKind, Hover, HoverContents, InlayHint, InlayHintLabel, Location, MarkupContent,
    MarkupKind, NumberOrString, ParameterInformation, ParameterLabel, SignatureHelp,
    SignatureInformation, TextEdit,
};
use stave_ide::{CompletionKind, Document, FoldKind, OutlineEntry, OutlineKind, SymbolKind};
use stave_lang::ast::Span;
use stave_lang::{Diagnostic, FileId, Severity};

use crate::line_index::{LineIndex, PositionEncoding};
use crate::uri::document_uri;

/// The `source` every diagnostic carries.
pub const DIAGNOSTIC_SOURCE: &str = "stave";

/// The command a "2 uses" lens runs: arguments are the document's URI and
/// the name's position.
pub const SHOW_USES_COMMAND: &str = "stave.showUses";

/// The command a "show in 3D" lens runs: the argument is the item's name.
pub const SHOW_IN_3D_COMMAND: &str = "stave.showIn3d";

/// The command a "show work card" lens runs: the argument is the part's
/// name.
pub const SHOW_WORK_CARD_COMMAND: &str = "stave.showWorkCard";

/// Where the text of a file a diagnostic names comes from: the document
/// and its text, as the diagnostic was found in.
pub trait Sources {
    /// The document and text of a file.
    fn source(&self, file: FileId) -> Option<(Document, &str)>;
}

/// A diagnostic as the protocol shows it, with the document its primary
/// label is in. The help line and notes follow the message, as the
/// command line prints them; the other labels become related
/// information.
pub fn diagnostic(
    found: &Diagnostic,
    sources: &dyn Sources,
    encoding: PositionEncoding,
) -> Option<(Document, lsp_types::Diagnostic)> {
    let primary = found.primary()?;
    let (document, text) = sources.source(primary.file)?;
    let range = LineIndex::new(text).range(primary.span, encoding)?;
    let mut message = found.message.clone();
    if !primary.message.is_empty() {
        message.push_str(": ");
        message.push_str(&primary.message);
    }
    for note in &found.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    if let Some(help) = &found.help {
        message.push_str("\nhelp: ");
        message.push_str(help);
    }
    let related: Vec<DiagnosticRelatedInformation> = found
        .labels
        .iter()
        .skip(1)
        .filter_map(|label| {
            let (document, text) = sources.source(label.file)?;
            Some(DiagnosticRelatedInformation {
                location: Location::new(
                    document_uri(&document)?,
                    LineIndex::new(text).range(label.span, encoding)?,
                ),
                message: if label.message.is_empty() {
                    "here".to_owned()
                } else {
                    label.message.clone()
                },
            })
        })
        .collect();
    Some((
        document,
        lsp_types::Diagnostic {
            range,
            severity: Some(severity(found.severity)),
            code: Some(NumberOrString::String(found.code.as_str().to_owned())),
            code_description: None,
            source: Some(DIAGNOSTIC_SOURCE.to_owned()),
            message,
            related_information: (!related.is_empty()).then_some(related),
            tags: None,
            data: None,
        },
    ))
}

fn severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Note => DiagnosticSeverity::INFORMATION,
    }
}

/// The protocol's kind for a completion.
fn completion_kind(kind: CompletionKind) -> CompletionItemKind {
    match kind {
        CompletionKind::PlacementTerm
        | CompletionKind::CutOperation
        | CompletionKind::Operation
        | CompletionKind::Step => CompletionItemKind::FUNCTION,
        CompletionKind::Selector | CompletionKind::Word | CompletionKind::Label => {
            CompletionItemKind::ENUM_MEMBER
        }
        CompletionKind::BaseKind => CompletionItemKind::KEYWORD,
        CompletionKind::Field | CompletionKind::Member => CompletionItemKind::FIELD,
        CompletionKind::Unit => CompletionItemKind::UNIT,
        CompletionKind::Item(kind) => item_kind(kind),
    }
}

fn item_kind(kind: SymbolKind) -> CompletionItemKind {
    match kind {
        SymbolKind::Module => CompletionItemKind::MODULE,
        SymbolKind::Component | SymbolKind::Kind | SymbolKind::Recipe => CompletionItemKind::CLASS,
        SymbolKind::Param | SymbolKind::Let => CompletionItemKind::CONSTANT,
        SymbolKind::Role => CompletionItemKind::TYPE_PARAMETER,
        SymbolKind::Member | SymbolKind::MadePart => CompletionItemKind::FIELD,
        SymbolKind::Label => CompletionItemKind::ENUM_MEMBER,
        SymbolKind::Stock
        | SymbolKind::Profile
        | SymbolKind::Scene
        | SymbolKind::Region
        | SymbolKind::Part
        | SymbolKind::Joint
        | SymbolKind::Hardware
        | SymbolKind::Assembly
        | SymbolKind::Require => CompletionItemKind::VARIABLE,
    }
}

/// Completions as protocol items, keeping their order through
/// `sortText`.
pub fn completions(
    found: &[stave_ide::Completion],
    index: &LineIndex<'_>,
    encoding: PositionEncoding,
) -> Vec<CompletionItem> {
    let width = found.len().to_string().len();
    found
        .iter()
        .enumerate()
        .filter_map(|(place, completion)| {
            let range = index.range(completion.replace, encoding)?;
            Some(CompletionItem {
                label: completion.label.clone(),
                kind: Some(completion_kind(completion.kind)),
                detail: (!completion.detail.is_empty()).then(|| completion.detail.clone()),
                documentation: completion.docs.clone().map(Documentation::String),
                sort_text: Some(format!("{place:0width$}")),
                text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                    range,
                    completion.label.clone(),
                ))),
                ..CompletionItem::default()
            })
        })
        .collect()
}

/// Signature help as the protocol shows it: each parameter's label is
/// its place in the signature's label, so the editor can highlight it.
pub fn signature_help(help: &stave_ide::SignatureHelp) -> SignatureHelp {
    let mut searched_from = 0usize;
    let parameters = help
        .parameters
        .iter()
        .map(|parameter| {
            let label = help
                .label
                .get(searched_from..)
                .and_then(|rest| {
                    let at = rest.find(&parameter.name)?.checked_add(searched_from)?;
                    let end = at.checked_add(parameter.name.len())?;
                    let start = utf16_length(help.label.get(..at)?)?;
                    let stop = utf16_length(help.label.get(..end)?)?;
                    searched_from = end;
                    Some(ParameterLabel::LabelOffsets([start, stop]))
                })
                .unwrap_or_else(|| ParameterLabel::Simple(parameter.name.clone()));
            let mut words = Vec::new();
            if let Some(ty) = &parameter.ty {
                words.push(ty.clone());
            }
            if let Some(default) = &parameter.default {
                words.push(format!("default {default}"));
            }
            if let Some(docs) = &parameter.docs {
                words.push(docs.clone());
            }
            ParameterInformation {
                label,
                documentation: (!words.is_empty())
                    .then(|| Documentation::String(words.join(" · "))),
            }
        })
        .collect();
    let active = help.active.and_then(|active| u32::try_from(active).ok());
    SignatureHelp {
        signatures: vec![SignatureInformation {
            label: help.label.clone(),
            documentation: help.docs.clone().map(Documentation::String),
            parameters: Some(parameters),
            active_parameter: active,
        }],
        active_signature: Some(0),
        active_parameter: active,
    }
}

/// Parameter label offsets are counted in UTF-16 units whatever the
/// position encoding.
fn utf16_length(text: &str) -> Option<u32> {
    u32::try_from(text.encode_utf16().count()).ok()
}

/// A hover as Markdown: the declaration in a code block, what it is, and
/// its doc comment.
pub fn hover(found: &stave_ide::Hover, index: &LineIndex<'_>, encoding: PositionEncoding) -> Hover {
    let mut value = format!("```stave\n{}\n```\n\n*{}*", found.signature, found.kind);
    if let Some(worked_out) = &found.value {
        value.push_str(" `");
        value.push_str(worked_out);
        value.push('`');
    }
    if let Some(docs) = &found.docs {
        value.push_str("\n\n");
        value.push_str(docs);
    }
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: index.range(found.span, encoding),
    }
}

/// The lenses above a file's declarations: "2 uses", and "show in 3D" on
/// what the 3D view can show.
pub fn code_lenses(
    lenses: &[stave_ide::Lens],
    document: &Document,
    index: &LineIndex<'_>,
    encoding: PositionEncoding,
) -> Vec<CodeLens> {
    let Some(uri) = document_uri(document) else {
        return Vec::new();
    };
    let mut shown = Vec::new();
    for lens in lenses {
        let Some(range) = index.range(lens.location.span, encoding) else {
            continue;
        };
        shown.push(CodeLens {
            range,
            command: Some(Command::new(
                lens.uses_text(),
                SHOW_USES_COMMAND.to_owned(),
                Some(vec![
                    serde_json::Value::String(uri.as_str().to_owned()),
                    serde_json::json!({
                        "line": range.start.line,
                        "character": range.start.character,
                    }),
                ]),
            )),
            data: None,
        });
        if lens.shows_in_3d() {
            shown.push(CodeLens {
                range,
                command: Some(Command::new(
                    "show in 3D".to_owned(),
                    SHOW_IN_3D_COMMAND.to_owned(),
                    Some(vec![serde_json::Value::String(lens.name.clone())]),
                )),
                data: None,
            });
        }
        if lens.shows_work_card() {
            shown.push(CodeLens {
                range,
                command: Some(Command::new(
                    "show work card".to_owned(),
                    SHOW_WORK_CARD_COMMAND.to_owned(),
                    Some(vec![serde_json::Value::String(lens.name.clone())]),
                )),
                data: None,
            });
        }
    }
    shown
}

/// Inlay hints as the protocol shows them: `= 844mm` just after the text,
/// with a space before it.
pub fn inlay_hints(
    hints: &[stave_ide::InlayHint],
    index: &LineIndex<'_>,
    encoding: PositionEncoding,
) -> Vec<InlayHint> {
    hints
        .iter()
        .filter_map(|hint| {
            let position = index.position(usize::try_from(hint.offset).ok()?, encoding)?;
            Some(InlayHint {
                position,
                label: InlayHintLabel::String(hint.label.clone()),
                kind: None,
                text_edits: None,
                tooltip: None,
                padding_left: Some(true),
                padding_right: None,
                data: None,
            })
        })
        .collect()
}

/// The protocol's kind for an outline entry.
fn symbol_kind(kind: OutlineKind) -> lsp_types::SymbolKind {
    match kind {
        OutlineKind::Header => lsp_types::SymbolKind::FILE,
        OutlineKind::Block => lsp_types::SymbolKind::NAMESPACE,
        OutlineKind::Item(kind) => match kind {
            SymbolKind::Module => lsp_types::SymbolKind::MODULE,
            SymbolKind::Component | SymbolKind::Kind | SymbolKind::Recipe => {
                lsp_types::SymbolKind::CLASS
            }
            SymbolKind::Param | SymbolKind::Let => lsp_types::SymbolKind::CONSTANT,
            SymbolKind::Role => lsp_types::SymbolKind::TYPE_PARAMETER,
            SymbolKind::Member | SymbolKind::MadePart | SymbolKind::Label => {
                lsp_types::SymbolKind::FIELD
            }
            SymbolKind::Stock | SymbolKind::Profile => lsp_types::SymbolKind::STRUCT,
            SymbolKind::Scene => lsp_types::SymbolKind::EVENT,
            SymbolKind::Region => lsp_types::SymbolKind::ARRAY,
            SymbolKind::Part | SymbolKind::Hardware | SymbolKind::Assembly => {
                lsp_types::SymbolKind::OBJECT
            }
            SymbolKind::Joint => lsp_types::SymbolKind::OPERATOR,
            SymbolKind::Require => lsp_types::SymbolKind::BOOLEAN,
        },
    }
}

/// A file's outline as nested document symbols.
pub fn document_symbols(
    entries: &[OutlineEntry],
    index: &LineIndex<'_>,
    encoding: PositionEncoding,
) -> Vec<DocumentSymbol> {
    entries
        .iter()
        .filter_map(|entry| {
            let range = index.range(entry.span, encoding)?;
            let selection_range = index.range(entry.name_span, encoding)?;
            #[expect(
                deprecated,
                reason = "lsp-types requires the deprecated field to be written"
            )]
            let symbol = DocumentSymbol {
                name: entry.name.clone(),
                detail: (!entry.detail.is_empty()).then(|| entry.detail.clone()),
                kind: symbol_kind(entry.kind),
                tags: None,
                deprecated: None,
                range,
                selection_range,
                children: Some(document_symbols(&entry.children, index, encoding)),
            };
            Some(symbol)
        })
        .collect()
}

/// Folds as protocol folding ranges, by line. A block keeps its closing
/// line showing; one that ends on the line after it starts has nothing to
/// fold and is left out.
pub fn folding_ranges(
    folds: &[stave_ide::Fold],
    index: &LineIndex<'_>,
    encoding: PositionEncoding,
) -> Vec<FoldingRange> {
    let mut ranges: Vec<FoldingRange> = folds
        .iter()
        .filter_map(|fold| {
            let range = index.range(fold.span, encoding)?;
            let (end_line, kind) = match fold.kind {
                FoldKind::Block => (range.end.line.checked_sub(1)?, None),
                FoldKind::Comment => (range.end.line, Some(FoldingRangeKind::Comment)),
                FoldKind::Imports => (range.end.line, Some(FoldingRangeKind::Imports)),
            };
            (end_line > range.start.line).then_some(FoldingRange {
                start_line: range.start.line,
                start_character: None,
                end_line,
                end_character: None,
                kind,
                collapsed_text: None,
            })
        })
        .collect();
    ranges.dedup_by(|later, earlier| {
        later.start_line == earlier.start_line && later.end_line == earlier.end_line
    });
    ranges
}

/// Byte-range edits of a text as protocol text edits.
pub fn text_edits(
    edits: &[stave_lang::TextEdit],
    index: &LineIndex<'_>,
    encoding: PositionEncoding,
) -> Option<Vec<TextEdit>> {
    edits
        .iter()
        .map(|edit| {
            let start = u32::try_from(edit.range.start).ok()?;
            let end = u32::try_from(edit.range.end).ok()?;
            Some(TextEdit::new(
                index.range(Span::new(start, end), encoding)?,
                edit.text.clone(),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_ide::{ParameterKind, SignatureParameter};
    use stave_lang::ast::Span;
    use stave_lang::{Code, Label, SourceMap};
    use std::path::PathBuf;

    struct Map(SourceMap);

    impl Sources for Map {
        fn source(&self, file: FileId) -> Option<(Document, &str)> {
            let source = self.0.get(file)?;
            Some((Document::File(PathBuf::from(source.path())), source.text()))
        }
    }

    #[test]
    fn a_diagnostic_carries_its_help_notes_and_other_labels() {
        let mut map = SourceMap::new();
        let text = "// é 🪚\npart door : Panel\n  mirror(door_l, about: x)\n";
        let file = map.add("/p/doors.stave", text).unwrap();
        let at = |needle: &str| {
            let start = u32::try_from(text.find(needle).unwrap()).unwrap();
            Span::new(start, start + u32::try_from(needle.len()).unwrap())
        };
        let found = Diagnostic::error(
            Code::UnknownName,
            "no part named `door_l`",
            Label::with_message(file, at("door_l"), "not declared"),
        )
        .with_label(Label::with_message(file, at("door"), "a similar name"))
        .with_label(Label::new(file, at("Panel")))
        .with_note("names are looked up in this component")
        .with_help("did you mean `door`?");
        let (document, shown) = diagnostic(&found, &Map(map), PositionEncoding::Utf16).unwrap();
        assert_eq!(document, Document::File(PathBuf::from("/p/doors.stave")));
        insta::assert_snapshot!(serde_json::to_string_pretty(&shown).unwrap());
    }

    #[test]
    fn parameter_labels_point_into_the_signature() {
        let parameter = |name: &str| SignatureParameter {
            name: name.to_owned(),
            kind: ParameterKind::Role,
            positional: true,
            ty: Some("Part".to_owned()),
            default: None,
            required: true,
            docs: None,
        };
        let help = stave_ide::SignatureHelp {
            label: "dado(piece, into, depth = into.thickness / 3)".to_owned(),
            callee: "dado".to_owned(),
            parameters: vec![parameter("piece"), parameter("into"), parameter("depth")],
            active: Some(1),
            docs: None,
        };
        let shown = signature_help(&help);
        let labels: Vec<ParameterLabel> = shown.signatures[0]
            .parameters
            .clone()
            .unwrap()
            .into_iter()
            .map(|p| p.label)
            .collect();
        assert_eq!(
            labels,
            [
                ParameterLabel::LabelOffsets([5, 10]),
                ParameterLabel::LabelOffsets([12, 16]),
                ParameterLabel::LabelOffsets([18, 23]),
            ]
        );
        assert_eq!(shown.active_parameter, Some(1));
    }
}
