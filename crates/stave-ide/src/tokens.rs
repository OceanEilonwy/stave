//! Semantic colouring: the namespace of every word (grammar, "What an
//! editor gets": item kinds, members, fixed-position call heads, enum
//! words, units), from the syntax tree and the resolved references.

use stave_lang::ast::{Span, TypeName};
use stave_lang::{FileId, SyntaxKind, SyntaxToken};

use crate::index::{Index, SymbolKind};
use crate::words;

/// How a run of source text is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TokenClass {
    /// A keyword or a contextual word in its special position: `assembly`,
    /// `import`, `as`, `mirror`.
    Keyword,
    /// `// …` and `/* … */`.
    Comment,
    /// `/// …`: a doc comment.
    DocComment,
    /// A string's text and quotes.
    Text,
    /// A number or a quantity: `4`, `18mm`, `110deg`.
    Number,
    /// A call head fixed by its position or a built-in function: `flush`,
    /// `tenon`, `overlay`, `mirror`'s `mid`.
    Function,
    /// A component, a kind or a type name: `FramePanelDoor`, `Region`.
    Type,
    /// A module's name: `casework`, `hw`.
    Module,
    /// A word of a closed list: `up`, `across`, `open`, `plywood`.
    EnumWord,
    /// A field name, a named argument or a member after `.`.
    Field,
    /// A name that means an item of this kind.
    Item(SymbolKind),
    /// Brackets, operators and separators.
    Punctuation,
    /// A character the lexer couldn't read.
    Error,
    /// Anything else: a name that resolves to nothing.
    Plain,
}

/// One coloured run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticToken {
    /// Where it is.
    pub span: Span,
    /// How it is coloured.
    pub class: TokenClass,
}

fn span_of(token: &SyntaxToken) -> Span {
    let range = token.text_range();
    Span::new(u32::from(range.start()), u32::from(range.end()))
}

/// The class of a word that names a symbol of this kind.
fn item_class(kind: SymbolKind) -> TokenClass {
    match kind {
        SymbolKind::Module => TokenClass::Module,
        SymbolKind::Component | SymbolKind::Kind => TokenClass::Type,
        SymbolKind::Recipe
        | SymbolKind::Stock
        | SymbolKind::Profile
        | SymbolKind::Scene
        | SymbolKind::Param
        | SymbolKind::Let
        | SymbolKind::Region
        | SymbolKind::Part
        | SymbolKind::Joint
        | SymbolKind::Hardware
        | SymbolKind::Assembly
        | SymbolKind::Require
        | SymbolKind::Role
        | SymbolKind::MadePart
        | SymbolKind::Member
        | SymbolKind::Label => TokenClass::Item(kind),
    }
}

impl Index {
    /// Every significant token of a file, coloured, in order.
    pub(crate) fn tokens(&self, file: FileId) -> Vec<SemanticToken> {
        let Some(data) = self.files.get(&file) else {
            return Vec::new();
        };
        let root = data.root();
        // References of this file, in order, to walk alongside the tokens.
        let first = self.references.partition_point(|r| r.file < file);
        let references = self
            .references
            .get(first..)
            .unwrap_or(&[])
            .iter()
            .take_while(|r| r.file == file);
        let mut references = references.peekable();
        let mut out = Vec::new();
        for token in root
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
        {
            let kind = token.kind();
            if kind == SyntaxKind::WHITESPACE || kind == SyntaxKind::NEWLINE {
                continue;
            }
            let span = span_of(&token);
            while references
                .peek()
                .is_some_and(|r| r.span.end() <= span.start() && r.span != span)
            {
                references.next();
            }
            let resolved = references
                .peek()
                .filter(|r| r.span == span)
                .and_then(|r| self.symbol(r.symbol))
                .map(|s| item_class(s.kind));
            let class = match resolved {
                Some(class) => class,
                None => classify(&token),
            };
            out.push(SemanticToken { span, class });
        }
        out
    }
}

/// The class of a token that names no symbol.
fn classify(token: &SyntaxToken) -> TokenClass {
    let kind = token.kind();
    match kind {
        SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT => return TokenClass::Comment,
        SyntaxKind::DOC => return TokenClass::DocComment,
        SyntaxKind::STRING_QUOTE | SyntaxKind::STRING_TEXT => return TokenClass::Text,
        SyntaxKind::INTERP_OPEN | SyntaxKind::INTERP_CLOSE => return TokenClass::Punctuation,
        SyntaxKind::QUANTITY | SyntaxKind::INT_NUMBER | SyntaxKind::DECIMAL_NUMBER => {
            return TokenClass::Number;
        }
        SyntaxKind::ERROR_TOKEN => return TokenClass::Error,
        _ => {}
    }
    if kind.punctuation().is_some() {
        return TokenClass::Punctuation;
    }
    if !kind.is_word() {
        return TokenClass::Plain;
    }
    let text = token.text();
    let Some(parent) = token.parent() else {
        return TokenClass::Plain;
    };
    match parent.kind() {
        // A word written as a name: a field, an argument, a member, a
        // declared name or a type.
        SyntaxKind::NAME => {
            let Some(holder) = parent.parent() else {
                return TokenClass::Plain;
            };
            let first_name = holder
                .children()
                .find(|c| c.kind() == SyntaxKind::NAME)
                .is_some_and(|n| n == parent);
            match holder.kind() {
                SyntaxKind::FIELD | SyntaxKind::ARG | SyntaxKind::MIRROR => TokenClass::Field,
                SyntaxKind::MEMBER_EXPR | SyntaxKind::REF | SyntaxKind::PATH if !first_name => {
                    TokenClass::Field
                }
                SyntaxKind::CALL => TokenClass::Function,
                SyntaxKind::PARAM | SyntaxKind::ROLE if !first_name => TokenClass::Type,
                _ if TypeName::from_word(text).is_some() && !first_name => TokenClass::Type,
                _ => TokenClass::Plain,
            }
        }
        SyntaxKind::NAME_REF => {
            let called = parent
                .parent()
                .is_some_and(|p| p.kind() == SyntaxKind::CALL_EXPR)
                && parent.prev_sibling().is_none();
            if called && (words::builtin_signature(text).is_some() || words::is_fixed_call(text)) {
                TokenClass::Function
            } else if words::is_enum_word(text) {
                TokenClass::EnumWord
            } else {
                TokenClass::Plain
            }
        }
        // A word directly in an item or entry: a keyword, or a contextual
        // word in its special position (`as`, `mirror`, `roles`, `cut`,
        // the base kind after `kind name :`).
        SyntaxKind::KIND if !kind.is_keyword() || kind == SyntaxKind::KW_JOINT => {
            let after_colon = token
                .siblings_with_tokens(rowan::Direction::Prev)
                .any(|s| s.kind() == SyntaxKind::COLON);
            if after_colon {
                TokenClass::Type
            } else {
                TokenClass::Keyword
            }
        }
        SyntaxKind::RECIPE if !kind.is_keyword() => TokenClass::Function,
        _ => TokenClass::Keyword,
    }
}
