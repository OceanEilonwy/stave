//! The semantic token legend: how [`TokenClass`] and [`TokenModifier`]
//! become the protocol's token types and modifiers, and how a file's
//! tokens are encoded (relative lines and columns, one line per token).
//!
//! The legend is fixed: editors (the VS Code extension in `editors/`)
//! map these names to colours. The crate docs list it.

use lsp_types::{SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend};
use stave_ide::{SymbolKind, TokenClass, TokenModifier, TokenModifiers};

use crate::line_index::{LineIndex, PositionEncoding};

/// A token type in the legend: the standard ones Stave uses, then its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenType {
    Keyword,
    Comment,
    String,
    Number,
    Operator,
    Function,
    Type,
    Namespace,
    EnumMember,
    Property,
    Variable,
    Unit,
    Component,
    Kind,
    Recipe,
    Stock,
    Profile,
    Scene,
    Param,
    Let,
    Region,
    Part,
    Joint,
    Hardware,
    Assembly,
    Require,
    Role,
    MadePart,
    Member,
    Label,
}

impl TokenType {
    /// Every type, in legend order: a token's type is its place here.
    const ALL: [TokenType; 30] = [
        TokenType::Keyword,
        TokenType::Comment,
        TokenType::String,
        TokenType::Number,
        TokenType::Operator,
        TokenType::Function,
        TokenType::Type,
        TokenType::Namespace,
        TokenType::EnumMember,
        TokenType::Property,
        TokenType::Variable,
        TokenType::Unit,
        TokenType::Component,
        TokenType::Kind,
        TokenType::Recipe,
        TokenType::Stock,
        TokenType::Profile,
        TokenType::Scene,
        TokenType::Param,
        TokenType::Let,
        TokenType::Region,
        TokenType::Part,
        TokenType::Joint,
        TokenType::Hardware,
        TokenType::Assembly,
        TokenType::Require,
        TokenType::Role,
        TokenType::MadePart,
        TokenType::Member,
        TokenType::Label,
    ];

    /// Its name in the legend.
    fn name(self) -> &'static str {
        match self {
            TokenType::Keyword => "keyword",
            TokenType::Comment => "comment",
            TokenType::String => "string",
            TokenType::Number => "number",
            TokenType::Operator => "operator",
            TokenType::Function => "function",
            TokenType::Type => "type",
            TokenType::Namespace => "namespace",
            TokenType::EnumMember => "enumMember",
            TokenType::Property => "property",
            TokenType::Variable => "variable",
            TokenType::Unit => "unit",
            TokenType::Component => "component",
            TokenType::Kind => "kind",
            TokenType::Recipe => "recipe",
            TokenType::Stock => "stock",
            TokenType::Profile => "profile",
            TokenType::Scene => "scene",
            TokenType::Param => "param",
            TokenType::Let => "let",
            TokenType::Region => "region",
            TokenType::Part => "part",
            TokenType::Joint => "joint",
            TokenType::Hardware => "hardware",
            TokenType::Assembly => "assembly",
            TokenType::Require => "require",
            TokenType::Role => "role",
            TokenType::MadePart => "madePart",
            TokenType::Member => "member",
            TokenType::Label => "label",
        }
    }

    /// Its index in the legend.
    fn index(self) -> u32 {
        let position = TokenType::ALL.iter().position(|t| *t == self).unwrap_or(0);
        u32::try_from(position).unwrap_or(0)
    }

    /// The type of an item's name.
    fn of_item(kind: SymbolKind) -> TokenType {
        match kind {
            SymbolKind::Module => TokenType::Namespace,
            SymbolKind::Component => TokenType::Component,
            SymbolKind::Kind => TokenType::Kind,
            SymbolKind::Recipe => TokenType::Recipe,
            SymbolKind::Stock => TokenType::Stock,
            SymbolKind::Profile => TokenType::Profile,
            SymbolKind::Scene => TokenType::Scene,
            SymbolKind::Param => TokenType::Param,
            SymbolKind::Let => TokenType::Let,
            SymbolKind::Region => TokenType::Region,
            SymbolKind::Part => TokenType::Part,
            SymbolKind::Joint => TokenType::Joint,
            SymbolKind::Hardware => TokenType::Hardware,
            SymbolKind::Assembly => TokenType::Assembly,
            SymbolKind::Require => TokenType::Require,
            SymbolKind::Role => TokenType::Role,
            SymbolKind::MadePart => TokenType::MadePart,
            SymbolKind::Member => TokenType::Member,
            SymbolKind::Label => TokenType::Label,
        }
    }

    /// The type a class is sent as; `None` for text left uncoloured.
    fn of_class(class: TokenClass, modifiers: TokenModifiers) -> Option<TokenType> {
        Some(match class {
            TokenClass::Keyword => TokenType::Keyword,
            TokenClass::Comment | TokenClass::DocComment => TokenType::Comment,
            TokenClass::Text => TokenType::String,
            TokenClass::Number => TokenType::Number,
            TokenClass::Unit => TokenType::Unit,
            TokenClass::Function => TokenType::Function,
            TokenClass::Type => TokenType::Type,
            TokenClass::Module => TokenType::Namespace,
            TokenClass::EnumWord => TokenType::EnumMember,
            TokenClass::Field => TokenType::Property,
            TokenClass::Item(kind) => TokenType::of_item(kind),
            TokenClass::Punctuation => TokenType::Operator,
            // A name that means nothing is sent only when a modifier says
            // something about it (that it is unresolved).
            TokenClass::Plain if !modifiers.is_empty() => TokenType::Variable,
            TokenClass::Plain | TokenClass::Error => return None,
        })
    }
}

/// The legend name of one of Stave's modifiers.
fn modifier_name(modifier: TokenModifier) -> &'static str {
    match modifier {
        TokenModifier::Declaration => "declaration",
        TokenModifier::Reference => "reference",
        TokenModifier::StandardLibrary => "defaultLibrary",
        TokenModifier::Unresolved => "unresolved",
        TokenModifier::SolvedParam => "solvedParam",
    }
}

/// The bit of `documentation`, after Stave's own modifiers.
fn documentation_bit() -> u32 {
    1 << TokenModifier::ALL.len()
}

/// The bits of a token's modifiers.
fn modifier_bits(class: TokenClass, modifiers: TokenModifiers) -> u32 {
    let own = TokenModifier::ALL
        .iter()
        .enumerate()
        .filter(|(_, modifier)| modifiers.contains(**modifier))
        .fold(0u32, |bits, (place, _)| bits | (1 << place));
    match class {
        TokenClass::DocComment => own | documentation_bit(),
        TokenClass::Keyword
        | TokenClass::Comment
        | TokenClass::Text
        | TokenClass::Number
        | TokenClass::Unit
        | TokenClass::Function
        | TokenClass::Type
        | TokenClass::Module
        | TokenClass::EnumWord
        | TokenClass::Field
        | TokenClass::Item(_)
        | TokenClass::Punctuation
        | TokenClass::Error
        | TokenClass::Plain => own,
    }
}

/// The legend the server announces: the token types, then the
/// modifiers (Stave's [`TokenModifier`]s, then `documentation` for doc
/// comments).
pub fn legend() -> SemanticTokensLegend {
    let mut token_modifiers: Vec<SemanticTokenModifier> = TokenModifier::ALL
        .iter()
        .map(|modifier| SemanticTokenModifier::new(modifier_name(*modifier)))
        .collect();
    token_modifiers.push(SemanticTokenModifier::DOCUMENTATION);
    SemanticTokensLegend {
        token_types: TokenType::ALL
            .iter()
            .map(|t| SemanticTokenType::new(t.name()))
            .collect(),
        token_modifiers,
    }
}

/// A file's tokens, encoded as the protocol sends them: each relative to
/// the one before, a token over several lines split into one per line.
pub fn encode(
    tokens: &[stave_ide::SemanticToken],
    index: &LineIndex<'_>,
    encoding: PositionEncoding,
) -> Vec<SemanticToken> {
    let mut encoded = Vec::with_capacity(tokens.len());
    let mut previous_line = 0u32;
    let mut previous_start = 0u32;
    for token in tokens {
        let Some(token_type) = TokenType::of_class(token.class, token.modifiers) else {
            continue;
        };
        let bits = modifier_bits(token.class, token.modifiers);
        for piece in index.split_lines(token.span) {
            let Ok(start) = usize::try_from(piece.start()) else {
                continue;
            };
            let Some(position) = index.position(start, encoding) else {
                continue;
            };
            let delta_line = position.line.saturating_sub(previous_line);
            let delta_start = if delta_line == 0 {
                position.character.saturating_sub(previous_start)
            } else {
                position.character
            };
            encoded.push(SemanticToken {
                delta_line,
                delta_start,
                length: index.length(piece, encoding),
                token_type: token_type.index(),
                token_modifiers_bitset: bits,
            });
            previous_line = position.line;
            previous_start = position.character;
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::ast::Span;

    #[test]
    fn the_legend_is_fixed() {
        let legend = legend();
        let types: Vec<&str> = legend.token_types.iter().map(|t| t.as_str()).collect();
        assert_eq!(
            types,
            [
                "keyword",
                "comment",
                "string",
                "number",
                "operator",
                "function",
                "type",
                "namespace",
                "enumMember",
                "property",
                "variable",
                "unit",
                "component",
                "kind",
                "recipe",
                "stock",
                "profile",
                "scene",
                "param",
                "let",
                "region",
                "part",
                "joint",
                "hardware",
                "assembly",
                "require",
                "role",
                "madePart",
                "member",
                "label",
            ]
        );
        let modifiers: Vec<&str> = legend.token_modifiers.iter().map(|m| m.as_str()).collect();
        assert_eq!(
            modifiers,
            [
                "declaration",
                "reference",
                "defaultLibrary",
                "unresolved",
                "solvedParam",
                "documentation",
            ]
        );
    }

    /// `editors/semantic-tokens.json` is the copy of the legend the editor
    /// configurations are checked against; it must be the one sent.
    #[test]
    fn the_legend_matches_the_editors_copy() {
        let copy: serde_json::Value =
            serde_json::from_str(include_str!("../../../editors/semantic-tokens.json"))
                .expect("editors/semantic-tokens.json is JSON");
        let names = |key: &str| -> Vec<String> {
            copy[key]
                .as_array()
                .expect("a list of names")
                .iter()
                .map(|name| name.as_str().expect("a name").to_owned())
                .collect()
        };
        let legend = legend();
        let types: Vec<String> = legend
            .token_types
            .iter()
            .map(|t| t.as_str().to_owned())
            .collect();
        let modifiers: Vec<String> = legend
            .token_modifiers
            .iter()
            .map(|m| m.as_str().to_owned())
            .collect();
        assert_eq!(types, names("tokenTypes"));
        assert_eq!(modifiers, names("tokenModifiers"));
    }

    #[test]
    fn tokens_are_relative_and_split_by_line() {
        let text = "x 18mm\n/* a\nb */ é y";
        let index = LineIndex::new(text);
        let token = |start: u32, end: u32, class: TokenClass, modifiers: TokenModifiers| {
            stave_ide::SemanticToken {
                span: Span::new(start, end),
                class,
                modifiers,
            }
        };
        let reference = TokenModifiers::NONE.with(TokenModifier::Reference);
        let y = u32::try_from(text.find('y').unwrap()).unwrap();
        let tokens = [
            token(0, 1, TokenClass::Item(SymbolKind::Part), reference),
            token(2, 4, TokenClass::Number, TokenModifiers::NONE),
            token(4, 6, TokenClass::Unit, TokenModifiers::NONE),
            token(7, 16, TokenClass::Comment, TokenModifiers::NONE),
            token(y, y + 1, TokenClass::Plain, TokenModifiers::NONE),
        ];
        let encoded = encode(&tokens, &index, PositionEncoding::Utf16);
        let rows: Vec<[u32; 5]> = encoded
            .iter()
            .map(|t| {
                [
                    t.delta_line,
                    t.delta_start,
                    t.length,
                    t.token_type,
                    t.token_modifiers_bitset,
                ]
            })
            .collect();
        let part = TokenType::Part.index();
        let unit = TokenType::Unit.index();
        let number = TokenType::Number.index();
        let comment = TokenType::Comment.index();
        assert_eq!(
            rows,
            [
                [0, 0, 1, part, 0b10],
                [0, 2, 2, number, 0],
                [0, 2, 2, unit, 0],
                [1, 0, 4, comment, 0],
                [1, 0, 4, comment, 0],
            ]
        );
    }
}
