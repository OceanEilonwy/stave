//! Converting between byte offsets, which Stave's spans are, and the
//! line and column positions the language server protocol uses (grammar,
//! "Positions": computed at the edge, never stored in the tree).
//!
//! A protocol column counts code units of an encoding the client and
//! server agree on at start-up ([`PositionEncoding`]): UTF-16 unless the
//! client offers UTF-8. `é` is one UTF-16 unit and two UTF-8 units; `🪚`
//! is two UTF-16 units and four UTF-8 units. A line ends at `\n`; a `\r`
//! before it belongs to the line break, not the line.

use lsp_types::{Position, PositionEncodingKind, Range, TextDocumentContentChangeEvent};
use stave_lang::ast::Span;

/// The unit a protocol column counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionEncoding {
    /// UTF-16 code units: what every client understands.
    #[default]
    Utf16,
    /// UTF-8 code units (bytes), when the client offers it.
    Utf8,
}

impl PositionEncoding {
    /// The encoding to use, from the encodings a client offers in its
    /// capabilities: UTF-8 when offered (no conversion needed), otherwise
    /// UTF-16.
    pub fn negotiate(offered: Option<&[PositionEncodingKind]>) -> PositionEncoding {
        if offered.is_some_and(|kinds| kinds.contains(&PositionEncodingKind::UTF8)) {
            PositionEncoding::Utf8
        } else {
            PositionEncoding::Utf16
        }
    }

    /// Its name in the protocol.
    pub fn kind(self) -> PositionEncodingKind {
        match self {
            PositionEncoding::Utf16 => PositionEncodingKind::UTF16,
            PositionEncoding::Utf8 => PositionEncodingKind::UTF8,
        }
    }

    /// How many units a character takes.
    fn units(self, character: char) -> u32 {
        let units = match self {
            PositionEncoding::Utf16 => character.len_utf16(),
            PositionEncoding::Utf8 => character.len_utf8(),
        };
        // A character is at most four bytes or two UTF-16 units.
        u32::try_from(units).unwrap_or(u32::MAX)
    }

    /// How many units a piece of text takes.
    fn length(self, text: &str) -> u32 {
        text.chars()
            .fold(0u32, |total, c| total.saturating_add(self.units(c)))
    }
}

/// Where every line of a text starts, for converting positions.
#[derive(Debug, Clone)]
pub struct LineIndex<'text> {
    text: &'text str,
    /// The byte offset of each line's first character; the first is 0.
    line_starts: Vec<usize>,
}

impl<'text> LineIndex<'text> {
    /// Indexes a text.
    pub fn new(text: &'text str) -> LineIndex<'text> {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at.saturating_add(1)))
            .collect();
        LineIndex { text, line_starts }
    }

    /// The text indexed.
    pub fn text(&self) -> &'text str {
        self.text
    }

    /// The byte range of a 0-based line, without its line break.
    fn line(&self, line: usize) -> Option<(usize, usize)> {
        let start = *self.line_starts.get(line)?;
        let end = match self.line_starts.get(line.saturating_add(1)) {
            Some(next) => next.saturating_sub(1),
            None => self.text.len(),
        };
        let content = self.text.get(start..end)?;
        let end = start.saturating_add(content.strip_suffix('\r').unwrap_or(content).len());
        Some((start, end))
    }

    /// The protocol position of a byte offset. `None` past the end of the
    /// text or inside a character.
    pub fn position(&self, offset: usize, encoding: PositionEncoding) -> Option<Position> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }
        let line = self
            .line_starts
            .partition_point(|start| *start <= offset)
            .checked_sub(1)?;
        let start = *self.line_starts.get(line)?;
        let column = encoding.length(self.text.get(start..offset)?);
        Some(Position::new(u32::try_from(line).ok()?, column))
    }

    /// The protocol range of a span. `None` when either end is outside the
    /// text.
    pub fn range(&self, span: Span, encoding: PositionEncoding) -> Option<Range> {
        let start = usize::try_from(span.start()).ok()?;
        let end = usize::try_from(span.end()).ok()?;
        Some(Range::new(
            self.position(start, encoding)?,
            self.position(end, encoding)?,
        ))
    }

    /// The byte offset of a protocol position. As the protocol asks, a
    /// line past the last is the end of the text and a column past a
    /// line's end is its end; a column inside a character (the middle of
    /// a UTF-16 surrogate pair) is the start of that character.
    pub fn offset(&self, position: Position, encoding: PositionEncoding) -> usize {
        let Ok(line) = usize::try_from(position.line) else {
            return self.text.len();
        };
        let Some((start, end)) = self.line(line) else {
            return self.text.len();
        };
        let mut remaining = position.character;
        let mut offset = start;
        for character in self.text.get(start..end).unwrap_or_default().chars() {
            let units = encoding.units(character);
            if remaining < units {
                break;
            }
            remaining = remaining.saturating_sub(units);
            offset = offset.saturating_add(character.len_utf8());
        }
        offset
    }

    /// The byte offset of a protocol position, as a span offset.
    pub fn span_offset(&self, position: Position, encoding: PositionEncoding) -> Option<u32> {
        u32::try_from(self.offset(position, encoding)).ok()
    }

    /// Every line a span covers, as one span per line without line
    /// breaks: how a multi-line token is sent to a client that takes
    /// tokens one line at a time. Empty pieces are left out.
    pub fn split_lines(&self, span: Span) -> Vec<Span> {
        let (Ok(start), Ok(end)) = (usize::try_from(span.start()), usize::try_from(span.end()))
        else {
            return Vec::new();
        };
        let mut pieces = Vec::new();
        let first = self
            .line_starts
            .partition_point(|line_start| *line_start <= start)
            .saturating_sub(1);
        let mut line = first;
        while let Some((line_start, line_end)) = self.line(line) {
            if line_start >= end && line != first {
                break;
            }
            let piece_start = start.max(line_start);
            let piece_end = end.min(line_end);
            if piece_start < piece_end
                && let (Ok(piece_start), Ok(piece_end)) =
                    (u32::try_from(piece_start), u32::try_from(piece_end))
            {
                pieces.push(Span::new(piece_start, piece_end));
            }
            line = line.saturating_add(1);
        }
        pieces
    }

    /// The length of a span's text in protocol units.
    pub fn length(&self, span: Span, encoding: PositionEncoding) -> u32 {
        let (Ok(start), Ok(end)) = (usize::try_from(span.start()), usize::try_from(span.end()))
        else {
            return 0;
        };
        self.text
            .get(start..end)
            .map_or(0, |piece| encoding.length(piece))
    }
}

/// Applies the changes of one `textDocument/didChange` notification to a
/// text, in order: a change with a range replaces that range, one without
/// replaces the whole text.
pub fn apply_changes(
    text: &str,
    changes: &[TextDocumentContentChangeEvent],
    encoding: PositionEncoding,
) -> String {
    let mut text = text.to_owned();
    for change in changes {
        match change.range {
            None => change.text.clone_into(&mut text),
            Some(range) => {
                let index = LineIndex::new(&text);
                let start = index.offset(range.start, encoding);
                let end = index.offset(range.end, encoding).max(start);
                text.replace_range(start..end, &change.text);
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "part a : oak\r\n// é width\n  length: 18mm // 🪚 saw\nend";

    #[test]
    fn ascii_offsets_round_trip() {
        let index = LineIndex::new(TEXT);
        let offset = TEXT.find("oak").unwrap();
        let position = index.position(offset, PositionEncoding::Utf16).unwrap();
        assert_eq!(position, Position::new(0, 9));
        assert_eq!(index.offset(position, PositionEncoding::Utf16), offset);
    }

    #[test]
    fn non_ascii_columns_count_code_units() {
        let index = LineIndex::new(TEXT);
        // After `é` (two bytes, one UTF-16 unit).
        let width = TEXT.find("width").unwrap();
        assert_eq!(
            index.position(width, PositionEncoding::Utf16),
            Some(Position::new(1, 5))
        );
        assert_eq!(
            index.position(width, PositionEncoding::Utf8),
            Some(Position::new(1, 6))
        );
        // After `🪚` (four bytes, two UTF-16 units).
        let saw = TEXT.find("saw").unwrap();
        let utf16 = index.position(saw, PositionEncoding::Utf16).unwrap();
        let utf8 = index.position(saw, PositionEncoding::Utf8).unwrap();
        assert_eq!(utf16, Position::new(2, 21));
        assert_eq!(utf8, Position::new(2, 23));
        assert_eq!(index.offset(utf16, PositionEncoding::Utf16), saw);
        assert_eq!(index.offset(utf8, PositionEncoding::Utf8), saw);
    }

    #[test]
    fn a_column_inside_a_surrogate_pair_is_the_character_start() {
        let index = LineIndex::new(TEXT);
        let saw = TEXT.find('🪚').unwrap();
        let column = index
            .position(saw, PositionEncoding::Utf16)
            .unwrap()
            .character;
        assert_eq!(
            index.offset(Position::new(2, column + 1), PositionEncoding::Utf16),
            saw
        );
    }

    #[test]
    fn offsets_inside_a_character_or_past_the_end_have_no_position() {
        let index = LineIndex::new(TEXT);
        let e = TEXT.find('é').unwrap();
        assert_eq!(index.position(e + 1, PositionEncoding::Utf16), None);
        assert_eq!(
            index.position(TEXT.len() + 1, PositionEncoding::Utf16),
            None
        );
        assert_eq!(
            index.position(TEXT.len(), PositionEncoding::Utf16),
            Some(Position::new(3, 3))
        );
    }

    #[test]
    fn positions_past_a_line_or_the_text_are_clamped() {
        let index = LineIndex::new(TEXT);
        // The carriage return belongs to the line break.
        assert_eq!(
            index.offset(Position::new(0, 99), PositionEncoding::Utf16),
            TEXT.find('\r').unwrap()
        );
        assert_eq!(
            index.offset(Position::new(99, 0), PositionEncoding::Utf16),
            TEXT.len()
        );
    }

    #[test]
    fn a_span_over_lines_splits_into_one_piece_per_line() {
        let text = "/* one\ntwo é\n\nthree */ x";
        let index = LineIndex::new(text);
        let end = u32::try_from(text.find(" x").unwrap()).unwrap();
        let pieces: Vec<&str> = index
            .split_lines(Span::new(0, end))
            .into_iter()
            .map(|span| &text[span.start() as usize..span.end() as usize])
            .collect();
        assert_eq!(pieces, ["/* one", "two é", "three */"]);
        let two = Span::new(7, 13);
        assert_eq!(index.length(two, PositionEncoding::Utf16), 5);
        assert_eq!(index.length(two, PositionEncoding::Utf8), 6);
    }

    #[test]
    fn changes_apply_in_order_in_code_units() {
        let text = "a é b\nc";
        let changes = [
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 4), Position::new(0, 5))),
                range_length: None,
                text: "🪚".to_owned(),
            },
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 6), Position::new(1, 0))),
                range_length: None,
                text: " ".to_owned(),
            },
        ];
        assert_eq!(
            apply_changes(text, &changes, PositionEncoding::Utf16),
            "a é 🪚 c"
        );
        let whole = [TextDocumentContentChangeEvent {
            range: None,
            range_length: None,
            text: "new".to_owned(),
        }];
        assert_eq!(apply_changes(text, &whole, PositionEncoding::Utf8), "new");
    }

    #[test]
    fn encodings_are_negotiated() {
        assert_eq!(PositionEncoding::negotiate(None), PositionEncoding::Utf16);
        let offered = [PositionEncodingKind::UTF16, PositionEncodingKind::UTF8];
        assert_eq!(
            PositionEncoding::negotiate(Some(&offered)),
            PositionEncoding::Utf8
        );
        assert_eq!(PositionEncoding::Utf8.kind(), PositionEncodingKind::UTF8);
    }
}
