//! Semantic colouring: the namespace of every word (grammar, "What an
//! editor gets": item kinds, members, fixed-position call heads, enum
//! words, units), from the syntax tree and the resolved references.
//!
//! Each [`SemanticToken`] carries a [`TokenClass`] (what the word is) and
//! a set of [`TokenModifiers`] (what this occurrence does: declares or
//! names an item, from the standard library or not, unresolved, a solved
//! param), the shape the language server protocol's semantic tokens take.
//! A quantity is two tokens, its number and its unit, so the unit can be
//! drawn apart from the number.

use stave_lang::ast::{Span, TypeName};
use stave_lang::{FileId, SyntaxKind, SyntaxToken};

use crate::index::{Index, SymbolKind, UseKind};
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
    /// A number, or the number of a quantity: `4`, `0.5`, the `18` of
    /// `18mm`, the `3/4` of `3/4in`.
    Number,
    /// The unit of a quantity: the `mm` of `18mm`, the `deg` of `110deg`.
    Unit,
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

/// What one occurrence of a word does, beside its [`TokenClass`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TokenModifier {
    /// The name an item is declared with: `side_l` in `part side_l : oak`.
    Declaration,
    /// A name that means an item declared elsewhere: `side_l` in
    /// `against(side_l)`.
    Reference,
    /// A name of an item from the standard library: `FramePanelDoor`,
    /// `dado`.
    StandardLibrary,
    /// A word where an item's name goes that means nothing: a typo, or a
    /// name not declared yet.
    Unresolved,
    /// A `solved param`, declared or named.
    SolvedParam,
}

impl TokenModifier {
    /// Every modifier, in a fixed order: the order a language server lists
    /// them in its legend.
    pub const ALL: [TokenModifier; 5] = [
        TokenModifier::Declaration,
        TokenModifier::Reference,
        TokenModifier::StandardLibrary,
        TokenModifier::Unresolved,
        TokenModifier::SolvedParam,
    ];

    /// Its name in lower case with underscores: `declaration`,
    /// `standard_library`.
    pub fn word(self) -> &'static str {
        match self {
            TokenModifier::Declaration => "declaration",
            TokenModifier::Reference => "reference",
            TokenModifier::StandardLibrary => "standard_library",
            TokenModifier::Unresolved => "unresolved",
            TokenModifier::SolvedParam => "solved_param",
        }
    }

    /// Its bit in a [`TokenModifiers`].
    fn bit(self) -> u8 {
        match self {
            TokenModifier::Declaration => 1,
            TokenModifier::Reference => 1 << 1,
            TokenModifier::StandardLibrary => 1 << 2,
            TokenModifier::Unresolved => 1 << 3,
            TokenModifier::SolvedParam => 1 << 4,
        }
    }
}

/// A set of [`TokenModifier`]s.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TokenModifiers(u8);

impl std::fmt::Debug for TokenModifiers {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_set().entries(self.iter()).finish()
    }
}

impl TokenModifiers {
    /// No modifiers.
    pub const NONE: TokenModifiers = TokenModifiers(0);

    /// `true` if the set holds `modifier`.
    pub fn contains(self, modifier: TokenModifier) -> bool {
        self.0 & modifier.bit() != 0
    }

    /// The set with `modifier` added.
    #[must_use]
    pub fn with(self, modifier: TokenModifier) -> TokenModifiers {
        TokenModifiers(self.0 | modifier.bit())
    }

    /// `true` if the set holds nothing.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The modifiers in the set, in the order of [`TokenModifier::ALL`].
    pub fn iter(self) -> impl Iterator<Item = TokenModifier> {
        TokenModifier::ALL
            .into_iter()
            .filter(move |modifier| self.contains(*modifier))
    }
}

impl FromIterator<TokenModifier> for TokenModifiers {
    fn from_iter<I: IntoIterator<Item = TokenModifier>>(modifiers: I) -> TokenModifiers {
        modifiers
            .into_iter()
            .fold(TokenModifiers::NONE, TokenModifiers::with)
    }
}

/// One coloured run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticToken {
    /// Where it is.
    pub span: Span,
    /// How it is coloured.
    pub class: TokenClass,
    /// What this occurrence does.
    pub modifiers: TokenModifiers,
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
        let mut references = self.references(file).iter().peekable();
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
                .and_then(|r| Some((r, self.symbol(r.symbol)?)));
            let Some((reference, symbol)) = resolved else {
                if kind == SyntaxKind::QUANTITY {
                    push_quantity(&mut out, span, token.text());
                    continue;
                }
                let modifiers = if names_nothing(&token) {
                    TokenModifiers::NONE.with(TokenModifier::Unresolved)
                } else {
                    TokenModifiers::NONE
                };
                out.push(SemanticToken {
                    span,
                    class: classify(&token),
                    modifiers,
                });
                continue;
            };
            let mut modifiers = TokenModifiers::NONE.with(match reference.kind {
                UseKind::Declared => TokenModifier::Declaration,
                UseKind::Imported
                | UseKind::Mirrored(_)
                | UseKind::Posed(_)
                | UseKind::SceneField(_, _)
                | UseKind::TypeOf(_)
                | UseKind::Joined(_)
                | UseKind::Holds(_)
                | UseKind::Placed(_)
                | UseKind::Field(_, _)
                | UseKind::Checked(_)
                | UseKind::Value(_)
                | UseKind::Text(_)
                | UseKind::Other => TokenModifier::Reference,
            });
            if self
                .modules
                .get(&symbol.module)
                .is_some_and(|module| module.standard)
            {
                modifiers = modifiers.with(TokenModifier::StandardLibrary);
            }
            if self.is_solved(symbol.id) {
                modifiers = modifiers.with(TokenModifier::SolvedParam);
            }
            out.push(SemanticToken {
                span,
                class: item_class(symbol.kind),
                modifiers,
            });
        }
        out
    }
}

/// Pushes a quantity as two tokens, its number and its unit, or as one
/// number if it has no unit to split off.
fn push_quantity(out: &mut Vec<SemanticToken>, span: Span, text: &str) {
    let split = stave_lang::quantity_unit_start(text)
        .and_then(|length| u32::try_from(length).ok())
        .and_then(|length| span.start().checked_add(length))
        .filter(|&middle| middle < span.end());
    let Some(middle) = split else {
        out.push(SemanticToken {
            span,
            class: TokenClass::Number,
            modifiers: TokenModifiers::NONE,
        });
        return;
    };
    out.push(SemanticToken {
        span: Span::new(span.start(), middle),
        class: TokenClass::Number,
        modifiers: TokenModifiers::NONE,
    });
    out.push(SemanticToken {
        span: Span::new(middle, span.end()),
        class: TokenClass::Unit,
        modifiers: TokenModifiers::NONE,
    });
}

/// `true` when an identifier that no reference covers stands where an
/// item's name goes (a bare name in an expression, the first name of a
/// reference or a path) and so means nothing: a typo, or a name not
/// declared yet. Enum words, built-in functions and fixed call heads are
/// words of the language, not unresolved names.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
)]
pub(crate) fn names_nothing(token: &SyntaxToken) -> bool {
    if token.kind() != SyntaxKind::IDENT {
        return false;
    }
    let text = token.text();
    let Some(parent) = token.parent() else {
        return false;
    };
    match parent.kind() {
        SyntaxKind::NAME_REF => {
            !(words::is_enum_word(text)
                || words::is_feature_word(text)
                || words::is_fixed_call(text)
                || words::builtin_signature(text).is_some()
                || text == "same")
        }
        SyntaxKind::NAME => parent.parent().is_some_and(|holder| {
            matches!(holder.kind(), SyntaxKind::REF | SyntaxKind::PATH)
                && holder
                    .children()
                    .find(|c| c.kind() == SyntaxKind::NAME)
                    .is_some_and(|first| first == parent)
        }),
        _ => false,
    }
}

/// The class of a token that names no symbol.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
)]
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
            } else if words::is_enum_word(text) || words::is_feature_word(text) {
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
