//! The lexer: source text to tokens, every byte accounted for.
//!
//! Lexing runs in two passes:
//!
//! 1. **Raw tokens.** Words, numbers and quantities (one token each, longest
//!    match first), punctuation, strings split into text and interpolation
//!    parts, comments, doc comments, whitespace and newlines. Characters that
//!    start no token become [`SyntaxKind::ERROR_TOKEN`]s, reported by the
//!    parser.
//! 2. **Newlines and brackets** (grammar, "Newlines" and "Unclosed
//!    brackets"). A newline is dropped (turned into whitespace trivia) inside
//!    `( )` and `[ ]` and after a continuation token. When a line starts with
//!    an item keyword that is not followed by `:`, every open `(` and `[` is
//!    closed with a zero-width closing token and reported as
//!    `error[unclosed-bracket]`. A `}` does the same for the brackets opened
//!    inside its block, since no expression can contain a `}`.
//!
//! The parser therefore never sees an unbalanced `(` or `[`, and a forgotten
//! `)` costs one error instead of the rest of the file.

use crate::ast::{Decimal, Fraction, Magnitude, Number, Quantity, Span, Unit};
use crate::diagnostics::{Code, Diagnostic, FileId, Label};
use crate::syntax::SyntaxKind;

/// One token: its kind and where it is. Zero-width tokens are closing
/// brackets inserted by the unclosed-bracket rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Token {
    pub kind: SyntaxKind,
    pub span: Span,
}

/// The lexer's output.
#[derive(Debug)]
pub(crate) struct Lexed {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes a whole file. `text` must be at most `u32::MAX` bytes long.
pub(crate) fn lex(file: FileId, text: &str) -> Lexed {
    let mut raw = RawLexer {
        file,
        text,
        bytes: text.as_bytes(),
        position: 0,
        tokens: Vec::with_capacity(text.len() / 3),
        diagnostics: Vec::new(),
    };
    raw.run();
    let RawLexer {
        tokens,
        mut diagnostics,
        ..
    } = raw;
    let tokens = cook(file, text, tokens, &mut diagnostics);
    Lexed {
        tokens,
        diagnostics,
    }
}

fn offset(position: usize) -> u32 {
    u32::try_from(position).unwrap_or(u32::MAX)
}

fn span(start: usize, end: usize) -> Span {
    Span::new(offset(start), offset(end))
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// How a token is described after "found" in a message.
pub(crate) fn describe_char(text: &str) -> String {
    match text.chars().next() {
        None => "end of file".to_owned(),
        Some('\n') => "end of line".to_owned(),
        Some(c) => format!("'{c}'"),
    }
}

struct RawLexer<'a> {
    file: FileId,
    text: &'a str,
    bytes: &'a [u8],
    position: usize,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
}

impl RawLexer<'_> {
    fn byte(&self, position: usize) -> u8 {
        self.bytes.get(position).copied().unwrap_or(0)
    }

    fn push(&mut self, kind: SyntaxKind, start: usize, end: usize) {
        self.tokens.push(Token {
            kind,
            span: span(start, end),
        });
    }

    fn error(&mut self, code: Code, message: String, at: Span, help: Option<String>) {
        let mut diagnostic = Diagnostic::error(code, message, Label::new(self.file, at));
        diagnostic.help = help;
        self.diagnostics.push(diagnostic);
    }

    /// Byte length of the character at `position`.
    fn char_length(&self, position: usize) -> usize {
        self.text
            .get(position..)
            .and_then(|rest| rest.chars().next())
            .map_or(1, char::len_utf8)
    }

    fn scan_while(&self, mut position: usize, keep: impl Fn(u8) -> bool) -> usize {
        while position < self.bytes.len() && keep(self.byte(position)) {
            position += 1;
        }
        position
    }

    /// End of an `INT` starting at `position`, which must be a digit.
    fn scan_int(&self, position: usize) -> usize {
        self.scan_while(position, |b| b.is_ascii_digit() || b == b'_')
    }

    fn run(&mut self) {
        // A byte-order mark some editors write is whitespace.
        if self.text.starts_with('\u{feff}') {
            self.position = '\u{feff}'.len_utf8();
            self.push(SyntaxKind::WHITESPACE, 0, self.position);
        }
        while self.position < self.bytes.len() {
            let start = self.position;
            let byte = self.byte(start);
            match byte {
                b'\n' => {
                    self.position += 1;
                    self.push(SyntaxKind::NEWLINE, start, self.position);
                }
                b' ' | b'\t' | b'\r' | 0x0b | 0x0c => {
                    self.position =
                        self.scan_while(start, |b| matches!(b, b' ' | b'\t' | b'\r' | 0x0b | 0x0c));
                    self.push(SyntaxKind::WHITESPACE, start, self.position);
                }
                b'/' if self.byte(start + 1) == b'/' => {
                    let kind = if self.byte(start + 2) == b'/' {
                        SyntaxKind::DOC
                    } else {
                        SyntaxKind::LINE_COMMENT
                    };
                    let mut end = self.scan_while(start, |b| b != b'\n');
                    if end > start && self.byte(end - 1) == b'\r' {
                        end -= 1;
                    }
                    self.position = end;
                    self.push(kind, start, end);
                }
                b'/' if self.byte(start + 1) == b'*' => self.block_comment(start),
                b'"' => self.string(start),
                b'0'..=b'9' => self.number(start),
                b if is_ident_start(b) => {
                    self.position = self.scan_while(start, is_ident_char);
                    let word = self.text.get(start..self.position).unwrap_or("");
                    let kind = SyntaxKind::keyword(word).unwrap_or(SyntaxKind::IDENT);
                    self.push(kind, start, self.position);
                }
                _ => self.punctuation(start),
            }
        }
    }

    fn block_comment(&mut self, start: usize) {
        let body = self.text.get(start + 2..).unwrap_or("");
        match body.find("*/") {
            Some(found) => {
                self.position = start + 2 + found + 2;
                self.push(SyntaxKind::BLOCK_COMMENT, start, self.position);
            }
            None => {
                self.position = self.bytes.len();
                self.push(SyntaxKind::BLOCK_COMMENT, start, self.position);
                self.error(
                    Code::Syntax,
                    "expected '*/', found end of file".to_owned(),
                    span(start, start + 2),
                    Some("close the comment with */".to_owned()),
                );
            }
        }
    }

    fn punctuation(&mut self, start: usize) {
        let next = self.byte(start + 1);
        let (kind, length) = match (self.byte(start), next) {
            (b'{', _) => (SyntaxKind::L_BRACE, 1),
            (b'}', _) => (SyntaxKind::R_BRACE, 1),
            (b'(', _) => (SyntaxKind::L_PAREN, 1),
            (b')', _) => (SyntaxKind::R_PAREN, 1),
            (b'[', _) => (SyntaxKind::L_BRACK, 1),
            (b']', _) => (SyntaxKind::R_BRACK, 1),
            (b',', _) => (SyntaxKind::COMMA, 1),
            (b':', _) => (SyntaxKind::COLON, 1),
            (b'.', b'.') => (SyntaxKind::DOT2, 2),
            (b'.', _) => (SyntaxKind::DOT, 1),
            (b'=', b'=') => (SyntaxKind::EQ2, 2),
            (b'=', _) => (SyntaxKind::EQ, 1),
            (b'!', b'=') => (SyntaxKind::NEQ, 2),
            (b'!', _) => (SyntaxKind::BANG, 1),
            (b'<', b'=') => (SyntaxKind::LE, 2),
            (b'<', _) => (SyntaxKind::LT, 1),
            (b'>', b'=') => (SyntaxKind::GE, 2),
            (b'>', _) => (SyntaxKind::GT, 1),
            (b'+', _) => (SyntaxKind::PLUS, 1),
            (b'-', _) => (SyntaxKind::MINUS, 1),
            (b'*', _) => (SyntaxKind::STAR, 1),
            (b'/', _) => (SyntaxKind::SLASH, 1),
            (b'&', b'&') => (SyntaxKind::AMP2, 2),
            (b'|', b'|') => (SyntaxKind::PIPE2, 2),
            _ => (SyntaxKind::ERROR_TOKEN, self.char_length(start)),
        };
        self.position = start + length;
        self.push(kind, start, self.position);
    }

    /// A number or a quantity, longest match first (grammar, "Quantities").
    fn number(&mut self, start: usize) {
        let int_end = self.scan_int(start);
        let followed_by_unit = |lexer: &Self, at: usize| is_ident_start(lexer.byte(at));

        // Mixed fraction: 1-1/2in
        if self.byte(int_end) == b'-' && self.byte(int_end + 1).is_ascii_digit() {
            let numerator_end = self.scan_int(int_end + 1);
            if self.byte(numerator_end) == b'/' && self.byte(numerator_end + 1).is_ascii_digit() {
                let denominator_end = self.scan_int(numerator_end + 1);
                if followed_by_unit(self, denominator_end) {
                    return self.quantity(start, denominator_end);
                }
            }
        }
        // Fraction: 3/4in
        if self.byte(int_end) == b'/' && self.byte(int_end + 1).is_ascii_digit() {
            let denominator_end = self.scan_int(int_end + 1);
            if followed_by_unit(self, denominator_end) {
                return self.quantity(start, denominator_end);
            }
        }
        // Decimal: 18.5mm or 0.5
        if self.byte(int_end) == b'.' && self.byte(int_end + 1).is_ascii_digit() {
            let fraction_end = self.scan_while(int_end + 1, |b| b.is_ascii_digit());
            if followed_by_unit(self, fraction_end) {
                return self.quantity(start, fraction_end);
            }
            return self.unitless(SyntaxKind::DECIMAL_NUMBER, start, fraction_end);
        }
        if followed_by_unit(self, int_end) {
            return self.quantity(start, int_end);
        }
        self.unitless(SyntaxKind::INT_NUMBER, start, int_end)
    }

    fn unitless(&mut self, kind: SyntaxKind, start: usize, end: usize) {
        self.position = end;
        self.push(kind, start, end);
        let text = self.text.get(start..end).unwrap_or("");
        if let Err(error) = parse_number(text) {
            self.literal_error(error, text, start, end);
        }
    }

    fn quantity(&mut self, start: usize, magnitude_end: usize) {
        let end = self.scan_while(magnitude_end, is_ident_char);
        self.position = end;
        self.push(SyntaxKind::QUANTITY, start, end);
        let text = self.text.get(start..end).unwrap_or("");
        if let Err(error) = parse_quantity(text) {
            self.literal_error(error, text, start, end);
        }
    }

    fn literal_error(&mut self, error: LiteralError, text: &str, start: usize, end: usize) {
        match error {
            LiteralError::UnknownUnit { magnitude, unit } => {
                let unit_start = start + magnitude.len();
                let help = suggest_unit(&unit).map(|unit| format!("write {magnitude}{unit}"));
                self.error(
                    Code::UnknownUnit,
                    format!("unknown unit '{unit}' (expected one of mm, cm, m, in, ft, deg)"),
                    span(unit_start, end),
                    help,
                );
            }
            LiteralError::ZeroDenominator => self.error(
                Code::Syntax,
                format!("expected a denominator other than zero, found '{text}'"),
                span(start, end),
                Some("a fraction can't be divided by zero".to_owned()),
            ),
            LiteralError::TooLarge => self.error(
                Code::Syntax,
                format!("expected a smaller number, found '{text}'"),
                span(start, end),
                Some("numbers must fit in 64 bits".to_owned()),
            ),
        }
    }

    /// A string, split into quote, text and interpolation tokens.
    fn string(&mut self, start: usize) {
        self.push(SyntaxKind::STRING_QUOTE, start, start + 1);
        let mut position = start + 1;
        let mut text_start = position;
        loop {
            match self.bytes.get(position).copied() {
                None | Some(b'\n') => {
                    self.flush_text(text_start, position);
                    let found = if position >= self.bytes.len() {
                        "end of file"
                    } else {
                        "end of line"
                    };
                    self.error(
                        Code::Syntax,
                        format!("expected '\"', found {found}"),
                        span(position, position),
                        Some("close the string with '\"' on the same line".to_owned()),
                    );
                    break;
                }
                Some(b'"') => {
                    self.flush_text(text_start, position);
                    self.push(SyntaxKind::STRING_QUOTE, position, position + 1);
                    position += 1;
                    break;
                }
                Some(b'\\') => position = self.escape(position),
                Some(b'{') => match self.scan_interp(position) {
                    Ok(parts) => {
                        self.flush_text(text_start, position);
                        position = parts.last().map_or(position + 1, |part| {
                            usize::try_from(part.span.end()).unwrap_or(position + 1)
                        });
                        self.tokens.extend(parts);
                        text_start = position;
                    }
                    Err((message, at)) => {
                        self.error(
                            Code::Syntax,
                            message,
                            at,
                            Some("write \\{ for a literal brace".to_owned()),
                        );
                        position += 1;
                    }
                },
                Some(_) => position += self.char_length(position),
            }
        }
        self.position = position;
    }

    fn flush_text(&mut self, start: usize, end: usize) {
        if end > start {
            self.push(SyntaxKind::STRING_TEXT, start, end);
        }
    }

    /// Checks the escape at `position` (a `\`) and returns where it ends.
    fn escape(&mut self, position: usize) -> usize {
        let next = position + 1;
        if next >= self.bytes.len() || self.byte(next) == b'\n' {
            self.error(
                Code::Syntax,
                "expected an escape (\\\" \\\\ \\n \\t \\{ or \\u{...}), found end of line"
                    .to_owned(),
                span(position, next),
                Some("write \\\\ for a backslash".to_owned()),
            );
            return next;
        }
        match self.byte(next) {
            b'"' | b'\\' | b'n' | b't' | b'{' => position + 2,
            b'u' if self.byte(position + 2) == b'{' => {
                let digits_start = position + 3;
                let digits_end = self.scan_while(digits_start, |b| b.is_ascii_hexdigit());
                let closed = self.byte(digits_end) == b'}';
                let valid = closed
                    && digits_end > digits_start
                    && self
                        .text
                        .get(digits_start..digits_end)
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .and_then(char::from_u32)
                        .is_some();
                let end = if closed {
                    digits_end + 1
                } else {
                    // Point at the whole `\u{...}` the user wrote.
                    let stop = self.scan_while(digits_start, |b| !matches!(b, b'}' | b'"' | b'\n'));
                    if self.byte(stop) == b'}' {
                        stop + 1
                    } else {
                        stop
                    }
                };
                if !valid {
                    let written = self.text.get(position..end).unwrap_or("\\u");
                    self.error(
                        Code::Syntax,
                        format!("expected a character code such as \\u{{e9}}, found '{written}'"),
                        span(position, end),
                        Some(
                            "write the character's hexadecimal code between the braces".to_owned(),
                        ),
                    );
                }
                end
            }
            _ => {
                let end = next + self.char_length(next);
                let written = self.text.get(position..end).unwrap_or("\\");
                self.error(
                    Code::Syntax,
                    format!(
                        "expected an escape (\\\" \\\\ \\n \\t \\{{ or \\u{{...}}), found '{written}'"
                    ),
                    span(position, end),
                    Some("write \\\\ for a backslash".to_owned()),
                );
                end
            }
        }
    }

    /// The tokens of `{name.member}` at `position`, or what went wrong.
    fn scan_interp(&self, position: usize) -> Result<Vec<Token>, (String, Span)> {
        let found = |at: usize| -> (String, Span) {
            let length = if at < self.bytes.len() {
                self.char_length(at)
            } else {
                0
            };
            let text = self.text.get(at..at + length).unwrap_or("");
            let description = match text {
                "\"" => "the end of the string".to_owned(),
                _ => describe_char(text),
            };
            (description, span(at, at + length))
        };
        let mut parts = vec![Token {
            kind: SyntaxKind::INTERP_OPEN,
            span: span(position, position + 1),
        }];
        let mut at = position + 1;
        loop {
            if !is_ident_start(self.byte(at)) {
                let (description, where_) = found(at);
                return Err((format!("expected a name, found {description}"), where_));
            }
            let end = self.scan_while(at, is_ident_char);
            let word = self.text.get(at..end).unwrap_or("");
            parts.push(Token {
                kind: SyntaxKind::keyword(word).unwrap_or(SyntaxKind::IDENT),
                span: span(at, end),
            });
            at = end;
            match self.byte(at) {
                b'.' => {
                    parts.push(Token {
                        kind: SyntaxKind::DOT,
                        span: span(at, at + 1),
                    });
                    at += 1;
                }
                b'}' => {
                    parts.push(Token {
                        kind: SyntaxKind::INTERP_CLOSE,
                        span: span(at, at + 1),
                    });
                    return Ok(parts);
                }
                _ => {
                    let (description, where_) = found(at);
                    return Err((format!("expected '}}' or '.', found {description}"), where_));
                }
            }
        }
    }
}

/// The unit a misspelt unit probably meant.
fn suggest_unit(written: &str) -> Option<&'static str> {
    let lower = written.to_ascii_lowercase();
    if let Some(unit) = Unit::from_suffix(&lower) {
        return Some(unit.suffix());
    }
    let unit = match lower.as_str() {
        "inch" | "inches" | "ins" => Unit::Inch,
        "foot" | "feet" | "fts" => Unit::Foot,
        "degree" | "degrees" | "degs" => Unit::Degree,
        "millimeter" | "millimeters" | "millimetre" | "millimetres" | "mms" => Unit::Millimeter,
        "centimeter" | "centimeters" | "centimetre" | "centimetres" | "cms" => Unit::Centimeter,
        "meter" | "meters" | "metre" | "metres" => Unit::Meter,
        _ => return None,
    };
    Some(unit.suffix())
}

// ---------------------------------------------------------------------------
// Literal values
// ---------------------------------------------------------------------------

/// Why a number or quantity literal has no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LiteralError {
    /// The letters after the number are not a unit.
    UnknownUnit {
        /// The number as written.
        magnitude: String,
        /// The letters as written.
        unit: String,
    },
    /// `3/0in`
    ZeroDenominator,
    /// More than 64 bits.
    TooLarge,
}

/// The value of an `INT` (underscores allowed after the first digit).
fn parse_int(text: &str) -> Result<u64, LiteralError> {
    let mut value: u64 = 0;
    for byte in text.bytes().filter(|&b| b != b'_') {
        let digit = u64::from(byte.wrapping_sub(b'0'));
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
            .ok_or(LiteralError::TooLarge)?;
    }
    Ok(value)
}

fn parse_decimal(whole: &str, fraction: &str) -> Result<Decimal, LiteralError> {
    let scale = u8::try_from(fraction.len()).map_err(|_| LiteralError::TooLarge)?;
    let mut digits = parse_int(whole)?;
    for byte in fraction.bytes() {
        digits = digits
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(byte.wrapping_sub(b'0'))))
            .ok_or(LiteralError::TooLarge)?;
    }
    Ok(Decimal { digits, scale })
}

/// Splits a magnitude (`19`, `18.5`, `3/4`, `1-1/2`) from what follows it.
fn parse_magnitude(text: &str) -> Result<(Magnitude, &str), LiteralError> {
    let bytes = text.as_bytes();
    let int_end = |from: usize| {
        bytes.get(from..).map_or(from, |rest| {
            from + rest
                .iter()
                .take_while(|b| b.is_ascii_digit() || **b == b'_')
                .count()
        })
    };
    let first_end = int_end(0);
    let first = text.get(..first_end).unwrap_or("");
    let at = |position: usize| bytes.get(position).copied().unwrap_or(0);
    let digit_at = |position: usize| at(position).is_ascii_digit();

    let fraction = |whole: u64, numerator: &str, denominator: &str| {
        let numerator = parse_int(numerator)?;
        let denominator = std::num::NonZeroU64::new(parse_int(denominator)?)
            .ok_or(LiteralError::ZeroDenominator)?;
        Ok::<_, LiteralError>(Magnitude::Fraction(Fraction {
            whole,
            numerator,
            denominator,
        }))
    };

    if at(first_end) == b'-' && digit_at(first_end + 1) {
        let numerator_end = int_end(first_end + 1);
        if at(numerator_end) == b'/' && digit_at(numerator_end + 1) {
            let denominator_end = int_end(numerator_end + 1);
            let magnitude = fraction(
                parse_int(first)?,
                text.get(first_end + 1..numerator_end).unwrap_or(""),
                text.get(numerator_end + 1..denominator_end).unwrap_or(""),
            )?;
            return Ok((magnitude, text.get(denominator_end..).unwrap_or("")));
        }
    }
    if at(first_end) == b'/' && digit_at(first_end + 1) {
        let denominator_end = int_end(first_end + 1);
        let magnitude = fraction(
            0,
            first,
            text.get(first_end + 1..denominator_end).unwrap_or(""),
        )?;
        return Ok((magnitude, text.get(denominator_end..).unwrap_or("")));
    }
    if at(first_end) == b'.' && digit_at(first_end + 1) {
        let fraction_end = first_end
            + 1
            + bytes.get(first_end + 1..).map_or(0, |rest| {
                rest.iter().take_while(|b| b.is_ascii_digit()).count()
            });
        let decimal = parse_decimal(first, text.get(first_end + 1..fraction_end).unwrap_or(""))?;
        return Ok((
            Magnitude::Decimal(decimal),
            text.get(fraction_end..).unwrap_or(""),
        ));
    }
    Ok((
        Magnitude::Integer(parse_int(first)?),
        text.get(first_end..).unwrap_or(""),
    ))
}

/// The value of a `QUANTITY` token's text.
pub(crate) fn parse_quantity(text: &str) -> Result<Quantity, LiteralError> {
    let (magnitude, unit_text) = parse_magnitude(text)?;
    let unit = Unit::from_suffix(unit_text).ok_or_else(|| LiteralError::UnknownUnit {
        magnitude: text
            .get(..text.len().saturating_sub(unit_text.len()))
            .unwrap_or("")
            .to_owned(),
        unit: unit_text.to_owned(),
    })?;
    Ok(Quantity { magnitude, unit })
}

/// The value of an `INT_NUMBER` or `DECIMAL_NUMBER` token's text.
pub(crate) fn parse_number(text: &str) -> Result<Number, LiteralError> {
    match parse_magnitude(text)? {
        (Magnitude::Integer(value), "") => Ok(Number::Integer(value)),
        (Magnitude::Decimal(decimal), "") => Ok(Number::Decimal(decimal)),
        // The lexer never produces a unitless fraction or trailing letters.
        (_, _) => Err(LiteralError::TooLarge),
    }
}

/// Resolves the escapes of a string's text part. Invalid escapes were
/// reported by the lexer and are kept as written.
pub(crate) fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('{') => out.push('{'),
            Some('u') => {
                chars.next();
                let mut hex = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    while let Some(&digit) = chars.peek() {
                        chars.next();
                        if digit == '}' {
                            break;
                        }
                        hex.push(digit);
                    }
                }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(decoded) => out.push(decoded),
                    None => {
                        out.push_str("\\u{");
                        out.push_str(&hex);
                        out.push('}');
                    }
                }
                continue;
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => {
                out.push('\\');
                continue;
            }
        }
        chars.next();
    }
    out
}

// ---------------------------------------------------------------------------
// Pass 2: newlines and brackets
// ---------------------------------------------------------------------------

/// 1-based line number of a byte offset.
fn line_of(text: &str, at: u32) -> usize {
    let end = usize::try_from(at).unwrap_or(usize::MAX).min(text.len());
    text.as_bytes()
        .get(..end)
        .map_or(0, |before| before.iter().filter(|&&b| b == b'\n').count())
        + 1
}

/// Why open brackets are being closed early.
#[derive(Clone, Copy)]
enum CloseReason {
    /// A line starts with an item keyword at this offset.
    Item(u32),
    /// A `}` at this offset ends the block.
    Brace(u32),
    /// The file ends.
    EndOfFile,
}

struct Cook<'a> {
    file: FileId,
    text: &'a str,
    out: Vec<Token>,
    /// Open `{`, `(` and `[`, innermost last.
    open: Vec<Token>,
    /// Index in `out` of the last significant token.
    last_significant: Option<usize>,
}

impl Cook<'_> {
    fn is_dropped_newline(&self, token: &Token) -> bool {
        token.kind == SyntaxKind::WHITESPACE
            && self
                .text
                .get(usize::try_from(token.span.start()).unwrap_or(usize::MAX)..)
                .is_some_and(|rest| rest.starts_with('\n'))
    }

    /// Closes the open `(` and `[` that `reason` ends: all of them for an
    /// item or the end of the file, the ones inside the innermost block for
    /// a `}`.
    fn close_brackets(&mut self, reason: CloseReason, diagnostics: &mut Vec<Diagnostic>) {
        let keep = match reason {
            CloseReason::Brace(_) => self
                .open
                .iter()
                .rposition(|token| token.kind == SyntaxKind::L_BRACE)
                .map_or(0, |index| index + 1),
            CloseReason::Item(_) | CloseReason::EndOfFile => 0,
        };
        let mut closing = Vec::new();
        let mut index = self.open.len();
        while index > keep {
            index -= 1;
            if let Some(token) = self.open.get(index).copied()
                && matches!(token.kind, SyntaxKind::L_PAREN | SyntaxKind::L_BRACK)
            {
                self.open.remove(index);
                closing.push(token);
            }
        }
        if closing.is_empty() {
            return;
        }
        let insert_at = self.last_significant.map_or(0, |index| index + 1);
        let at = self
            .last_significant
            .and_then(|index| self.out.get(index))
            .map_or(0, |token| token.span.end());
        let last_line = line_of(self.text, at);
        for (offset, opener) in closing.iter().enumerate() {
            let (closer, bracket, closer_text) = if opener.kind == SyntaxKind::L_PAREN {
                (SyntaxKind::R_PAREN, "(", ")")
            } else {
                (SyntaxKind::R_BRACK, "[", "]")
            };
            self.out.insert(
                insert_at + offset,
                Token {
                    kind: closer,
                    span: Span::new(at, at),
                },
            );
            let note = match reason {
                CloseReason::Item(next) => format!(
                    "the next declaration starts on line {}",
                    line_of(self.text, next)
                ),
                CloseReason::Brace(brace) => format!(
                    "the block it is in ends on line {}",
                    line_of(self.text, brace)
                ),
                CloseReason::EndOfFile => "the file ends before it is closed".to_owned(),
            };
            diagnostics.push(
                Diagnostic::error(
                    Code::UnclosedBracket,
                    format!("this \"{bracket}\" is never closed"),
                    Label::new(self.file, opener.span),
                )
                .with_note(note)
                .with_help(format!(
                    "add \"{closer_text}\" at the end of line {last_line}"
                )),
            );
        }
        // The newlines after the inserted closers end lines again.
        let first_after = insert_at + closing.len();
        for index in first_after..self.out.len() {
            if let Some(token) = self.out.get(index).copied()
                && self.is_dropped_newline(&token)
                && let Some(slot) = self.out.get_mut(index)
            {
                slot.kind = SyntaxKind::NEWLINE;
            }
        }
        self.last_significant = Some(insert_at + closing.len() - 1);
    }
}

fn cook(
    file: FileId,
    text: &str,
    raw: Vec<Token>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Token> {
    let mut cook = Cook {
        file,
        text,
        out: Vec::with_capacity(raw.len() + 4),
        open: Vec::new(),
        last_significant: None,
    };
    let mut previous: Option<SyntaxKind> = None;
    let mut at_line_start = true;

    for (index, token) in raw.iter().copied().enumerate() {
        match token.kind {
            kind if kind.is_trivia() => cook.out.push(token),
            SyntaxKind::NEWLINE => {
                let inside_brackets = cook.open.last().is_some_and(|open| {
                    matches!(open.kind, SyntaxKind::L_PAREN | SyntaxKind::L_BRACK)
                });
                let dropped = inside_brackets || previous.is_some_and(SyntaxKind::continues_line);
                cook.out.push(Token {
                    kind: if dropped {
                        SyntaxKind::WHITESPACE
                    } else {
                        SyntaxKind::NEWLINE
                    },
                    span: token.span,
                });
                at_line_start = true;
            }
            kind => {
                let has_open_bracket = || {
                    cook.open
                        .iter()
                        .any(|open| matches!(open.kind, SyntaxKind::L_PAREN | SyntaxKind::L_BRACK))
                };
                if at_line_start && kind.is_item_keyword() && has_open_bracket() {
                    let next = raw
                        .get(index + 1..)
                        .unwrap_or(&[])
                        .iter()
                        .find(|next| !next.kind.is_trivia() && next.kind != SyntaxKind::NEWLINE)
                        .map(|next| next.kind);
                    if next != Some(SyntaxKind::COLON) {
                        cook.close_brackets(CloseReason::Item(token.span.start()), diagnostics);
                    }
                }
                match kind {
                    SyntaxKind::L_BRACE | SyntaxKind::L_PAREN | SyntaxKind::L_BRACK => {
                        cook.open.push(token);
                    }
                    SyntaxKind::R_PAREN | SyntaxKind::R_BRACK => {
                        let opener = if kind == SyntaxKind::R_PAREN {
                            SyntaxKind::L_PAREN
                        } else {
                            SyntaxKind::L_BRACK
                        };
                        if cook.open.last().is_some_and(|open| open.kind == opener) {
                            cook.open.pop();
                        }
                    }
                    SyntaxKind::R_BRACE => {
                        cook.close_brackets(CloseReason::Brace(token.span.start()), diagnostics);
                        if cook
                            .open
                            .last()
                            .is_some_and(|open| open.kind == SyntaxKind::L_BRACE)
                        {
                            cook.open.pop();
                        }
                    }
                    _ => {}
                }
                cook.out.push(token);
                cook.last_significant = Some(cook.out.len() - 1);
                previous = Some(kind);
                at_line_start = false;
            }
        }
    }
    cook.close_brackets(CloseReason::EndOfFile, diagnostics);
    cook.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::SourceMap;
    use SyntaxKind::*;

    fn lex_text(text: &str) -> Lexed {
        let mut sources = SourceMap::new();
        let file = sources.add("test.stave", text).unwrap();
        lex(file, text)
    }

    /// Significant tokens with their text; dropped newlines are left out.
    fn significant(text: &str) -> Vec<(SyntaxKind, String)> {
        lex_text(text)
            .tokens
            .iter()
            .filter(|token| !token.kind.is_trivia())
            .map(|token| {
                let range = usize::try_from(token.span.start()).unwrap()
                    ..usize::try_from(token.span.end()).unwrap();
                (token.kind, text[range].to_owned())
            })
            .collect()
    }

    fn kinds(text: &str) -> Vec<SyntaxKind> {
        significant(text)
            .into_iter()
            .map(|(kind, _)| kind)
            .collect()
    }

    fn messages(text: &str) -> Vec<String> {
        lex_text(text)
            .diagnostics
            .iter()
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect()
    }

    #[test]
    fn every_byte_is_covered() {
        let text =
            "stave 1\n/// doc\npart a : s { length: 3/4in + \"x {y.z} \\{\" } // c\n/* b */ @";
        let tokens = lex_text(text).tokens;
        let mut at = 0;
        for token in &tokens {
            assert_eq!(token.span.start(), at, "{token:?}");
            at = token.span.end();
        }
        assert_eq!(usize::try_from(at).unwrap(), text.len());
    }

    #[test]
    fn quantity_forms() {
        let cases: &[(&str, Quantity)] = &[
            (
                "19mm",
                Quantity {
                    magnitude: Magnitude::Integer(19),
                    unit: Unit::Millimeter,
                },
            ),
            (
                "18.5mm",
                Quantity {
                    magnitude: Magnitude::Decimal(Decimal {
                        digits: 185,
                        scale: 1,
                    }),
                    unit: Unit::Millimeter,
                },
            ),
            (
                "3/4in",
                Quantity {
                    magnitude: Magnitude::Fraction(Fraction {
                        whole: 0,
                        numerator: 3,
                        denominator: std::num::NonZeroU64::new(4).unwrap(),
                    }),
                    unit: Unit::Inch,
                },
            ),
            (
                "1-1/2in",
                Quantity {
                    magnitude: Magnitude::Fraction(Fraction {
                        whole: 1,
                        numerator: 1,
                        denominator: std::num::NonZeroU64::new(2).unwrap(),
                    }),
                    unit: Unit::Inch,
                },
            ),
            (
                "1_000mm",
                Quantity {
                    magnitude: Magnitude::Integer(1000),
                    unit: Unit::Millimeter,
                },
            ),
            (
                "2cm",
                Quantity {
                    magnitude: Magnitude::Integer(2),
                    unit: Unit::Centimeter,
                },
            ),
            (
                "1m",
                Quantity {
                    magnitude: Magnitude::Integer(1),
                    unit: Unit::Meter,
                },
            ),
            (
                "3ft",
                Quantity {
                    magnitude: Magnitude::Integer(3),
                    unit: Unit::Foot,
                },
            ),
            (
                "110deg",
                Quantity {
                    magnitude: Magnitude::Integer(110),
                    unit: Unit::Degree,
                },
            ),
        ];
        for (text, expected) in cases {
            assert_eq!(
                significant(text),
                [(QUANTITY, (*text).to_owned())],
                "{text}"
            );
            assert_eq!(parse_quantity(text).as_ref(), Ok(expected), "{text}");
            assert!(messages(text).is_empty(), "{text}");
        }
        assert_eq!(
            parse_number("0.5"),
            Ok(Number::Decimal(Decimal {
                digits: 5,
                scale: 1
            }))
        );
        assert_eq!(parse_number("4"), Ok(Number::Integer(4)));
    }

    #[test]
    fn the_disambiguation_table() {
        let tokens = |text: &str| significant(text);
        let token = |kind: SyntaxKind, text: &str| (kind, text.to_owned());
        assert_eq!(tokens("3/4in"), [token(QUANTITY, "3/4in")]);
        assert_eq!(
            tokens("3/4"),
            [
                token(INT_NUMBER, "3"),
                token(SLASH, "/"),
                token(INT_NUMBER, "4")
            ]
        );
        assert_eq!(tokens("1-1/2in"), [token(QUANTITY, "1-1/2in")]);
        assert_eq!(
            tokens("1 - 1/2in"),
            [
                token(INT_NUMBER, "1"),
                token(MINUS, "-"),
                token(QUANTITY, "1/2in")
            ]
        );
        assert_eq!(
            tokens("2in-1/2in"),
            [
                token(QUANTITY, "2in"),
                token(MINUS, "-"),
                token(QUANTITY, "1/2in")
            ]
        );
        assert_eq!(tokens("1/3in"), [token(QUANTITY, "1/3in")]);
        assert_eq!(tokens("3inch"), [token(QUANTITY, "3inch")]);
        assert_eq!(
            messages("3inch"),
            ["unknown-unit: unknown unit 'inch' (expected one of mm, cm, m, in, ft, deg)"]
        );
        assert_eq!(
            lex_text("3inch").diagnostics[0].help.as_deref(),
            Some("write 3in")
        );
        assert_eq!(
            tokens("1..2"),
            [
                token(INT_NUMBER, "1"),
                token(DOT2, ".."),
                token(INT_NUMBER, "2")
            ]
        );
        assert_eq!(
            tokens("600mm..1200mm"),
            [
                token(QUANTITY, "600mm"),
                token(DOT2, ".."),
                token(QUANTITY, "1200mm")
            ]
        );
        assert_eq!(tokens("0.5"), [token(DECIMAL_NUMBER, "0.5")]);
        assert_eq!(
            tokens("1-1/2"),
            [
                token(INT_NUMBER, "1"),
                token(MINUS, "-"),
                token(INT_NUMBER, "1"),
                token(SLASH, "/"),
                token(INT_NUMBER, "2")
            ]
        );
        assert_eq!(
            messages("3/0in"),
            ["syntax: expected a denominator other than zero, found '3/0in'"]
        );
        assert_eq!(
            messages("99999999999999999999mm"),
            ["syntax: expected a smaller number, found '99999999999999999999mm'"]
        );
        assert_eq!(
            messages("2x"),
            ["unknown-unit: unknown unit 'x' (expected one of mm, cm, m, in, ft, deg)"]
        );
    }

    #[test]
    fn a_byte_order_mark_is_whitespace() {
        assert_eq!(kinds("\u{feff}stave 1"), [KW_STAVE, INT_NUMBER]);
        assert!(messages("\u{feff}stave 1").is_empty());
    }

    #[test]
    fn punctuation_and_words() {
        assert_eq!(
            kinds("a.b .. == != <= >= < > && || ! + - * / = , : { } ( ) [ ]"),
            [
                IDENT, DOT, IDENT, DOT2, EQ2, NEQ, LE, GE, LT, GT, AMP2, PIPE2, BANG, PLUS, MINUS,
                STAR, SLASH, EQ, COMMA, COLON, L_BRACE, R_BRACE, L_PAREN, R_PAREN, L_BRACK,
                R_BRACK
            ]
        );
        assert_eq!(
            kinds("part top as within"),
            [KW_PART, IDENT, IDENT, KW_WITHIN]
        );
        assert_eq!(kinds("& @ é"), [ERROR_TOKEN, ERROR_TOKEN, ERROR_TOKEN]);
    }

    #[test]
    fn comments_and_docs() {
        assert_eq!(
            lex_text("// c\n/// d\n/* b\n */x")
                .tokens
                .iter()
                .map(|token| token.kind)
                .collect::<Vec<_>>(),
            [LINE_COMMENT, NEWLINE, DOC, WHITESPACE, BLOCK_COMMENT, IDENT]
        );
        assert_eq!(
            messages("/* never closed"),
            ["syntax: expected '*/', found end of file"]
        );
    }

    #[test]
    fn newlines_are_dropped_inside_brackets() {
        assert_eq!(
            kinds("f(\na\n,\nb\n)\nx"),
            [IDENT, L_PAREN, IDENT, COMMA, IDENT, R_PAREN, NEWLINE, IDENT]
        );
        assert_eq!(
            kinds("[\n[\na\n]\n]\n"),
            [L_BRACK, L_BRACK, IDENT, R_BRACK, R_BRACK, NEWLINE]
        );
    }

    #[test]
    fn newlines_are_dropped_after_continuation_tokens() {
        for token in crate::LINE_CONTINUATION_TOKENS {
            let text = match *token {
                "DOC" => "/// doc\nx".to_owned(),
                "(" => "a (\nx)".to_owned(),
                "[" => "a [\nx]".to_owned(),
                other => format!("a {other}\nx"),
            };
            assert!(
                !kinds(&text).contains(&NEWLINE),
                "a newline after {token:?} must be dropped"
            );
        }
        // Comments between the token and the newline don't count.
        assert!(!kinds("a + // c\n/* d */\nb").contains(&NEWLINE));
        // Blank lines after a continuation are dropped too.
        assert!(!kinds("a,\n\n\nb").contains(&NEWLINE));
    }

    #[test]
    fn newlines_are_kept_everywhere_else() {
        for text in [
            "a :\nb", "a =\nb", "{\nb", "a\n}", "a.\nb", "a\n- b", "a )\nb",
        ] {
            assert!(kinds(text).contains(&NEWLINE), "{text:?}");
        }
    }

    #[test]
    fn strings_split_into_parts_with_spans() {
        let text = r#""Halve {first} and {piece.thickness} \{x}""#;
        let tokens: Vec<(SyntaxKind, &str)> = lex_text(text)
            .tokens
            .iter()
            .map(|token| {
                let range = usize::try_from(token.span.start()).unwrap()
                    ..usize::try_from(token.span.end()).unwrap();
                (token.kind, &text[range])
            })
            .collect();
        assert_eq!(
            tokens,
            [
                (STRING_QUOTE, "\""),
                (STRING_TEXT, "Halve "),
                (INTERP_OPEN, "{"),
                (IDENT, "first"),
                (INTERP_CLOSE, "}"),
                (STRING_TEXT, " and "),
                (INTERP_OPEN, "{"),
                (IDENT, "piece"),
                (DOT, "."),
                (IDENT, "thickness"),
                (INTERP_CLOSE, "}"),
                (STRING_TEXT, " \\{x}"),
                (STRING_QUOTE, "\""),
            ]
        );
        assert_eq!(unescape(" \\{x} \\u{e9} \\\" \\n"), " {x} é \" \n");
    }

    #[test]
    fn string_errors() {
        assert_eq!(
            messages("\"open\nx"),
            ["syntax: expected '\"', found end of line"]
        );
        assert_eq!(messages("\"{ x}\""), ["syntax: expected a name, found ' '"]);
        assert_eq!(
            messages("\"{x y}\""),
            ["syntax: expected '}' or '.', found ' '"]
        );
        assert_eq!(
            messages("\"\\q\""),
            ["syntax: expected an escape (\\\" \\\\ \\n \\t \\{ or \\u{...}), found '\\q'"]
        );
        assert_eq!(
            messages("\"\\u{zz}\""),
            ["syntax: expected a character code such as \\u{e9}, found '\\u{zz}'"]
        );
        // A failed interpolation is text, so the parser sees a plain string.
        assert_eq!(kinds("\"{ x}\""), [STRING_QUOTE, STRING_TEXT, STRING_QUOTE]);
    }

    #[test]
    fn an_item_keyword_closes_open_brackets() {
        let text = "region a = opening(\n  left: x,\n  top: y\nregion b = c\n";
        let lexed = lex_text(text);
        let kinds: Vec<SyntaxKind> = lexed
            .tokens
            .iter()
            .filter(|t| !t.kind.is_trivia())
            .map(|t| t.kind)
            .collect();
        assert_eq!(
            kinds,
            [
                KW_REGION, IDENT, EQ, IDENT, L_PAREN, IDENT, COLON, IDENT, COMMA, IDENT, COLON,
                IDENT, R_PAREN, NEWLINE, KW_REGION, IDENT, EQ, IDENT, NEWLINE
            ]
        );
        let closer = lexed.tokens.iter().find(|t| t.kind == R_PAREN).unwrap();
        assert!(closer.span.is_empty());
        assert_eq!(closer.span.start(), 39, "right after 'y'");
        let [diagnostic] = lexed.diagnostics.as_slice() else {
            panic!("{:?}", lexed.diagnostics);
        };
        assert_eq!(diagnostic.code, Code::UnclosedBracket);
        assert_eq!(diagnostic.message, "this \"(\" is never closed");
        assert_eq!(diagnostic.labels[0].span, Span::new(18, 19));
        assert_eq!(diagnostic.notes, ["the next declaration starts on line 4"]);
        assert_eq!(
            diagnostic.help.as_deref(),
            Some("add \")\" at the end of line 3")
        );
    }

    #[test]
    fn a_keyword_used_as_an_argument_name_does_not_close_brackets() {
        let text = "f(\nstock: oak,\npart: x\n)\n";
        assert!(lex_text(text).diagnostics.is_empty());
        assert_eq!(kinds(text).iter().filter(|k| **k == NEWLINE).count(), 1);
    }

    #[test]
    fn a_closing_brace_and_the_end_of_file_close_brackets() {
        let lexed = lex_text("a {\n  b: f(1,\n}\n");
        assert_eq!(lexed.diagnostics.len(), 1);
        assert_eq!(
            lexed.diagnostics[0].notes,
            ["the block it is in ends on line 3"]
        );
        let lexed = lex_text("x: [1, (2");
        assert_eq!(lexed.diagnostics.len(), 2);
        let closers: Vec<SyntaxKind> = lexed
            .tokens
            .iter()
            .filter(|t| t.span.is_empty())
            .map(|t| t.kind)
            .collect();
        assert_eq!(closers, [R_PAREN, R_BRACK]);
    }
}
