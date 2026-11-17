//! The lossless concrete syntax tree.
//!
//! The tree is built with [`rowan`], the green/red tree library used by
//! rust-analyzer:
//!
//! - **Green nodes** ([`GreenNode`]) are immutable, deduplicated and
//!   `Send + Sync`, so a parse result can be cached and shared between
//!   threads (the language server re-uses unchanged subtrees).
//! - **Red nodes** ([`SyntaxNode`]) are cheap cursors created on demand with
//!   parent pointers and absolute offsets, so an editor feature can walk up
//!   from the token under the cursor.
//! - **Every byte is kept**: whitespace, newlines, comments and the pieces
//!   of erroneous code are tokens in the tree, so `node.to_string()` gives
//!   back the source exactly. The formatter and later code actions edit this
//!   tree and never lose a comment.
//!
//! A hand-rolled green tree would need the same machinery (interning, offset
//! computation, parent pointers); `rowan` is small, well tested and has no
//! dependencies that matter here, so it is used instead of re-inventing it.
//!
//! The kinds below are shared by the lexer (token kinds) and the parser (node
//! kinds). [`crate::ast`] is a typed, owned view built from this tree.

use rowan::Language;

macro_rules! syntax_kinds {
    ($( $(#[doc = $doc:literal])* $name:ident ),* $(,)?) => {
        /// Every kind of token and node in the concrete syntax tree.
        #[allow(non_camel_case_types, clippy::upper_case_acronyms)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum SyntaxKind {
            $( $(#[doc = $doc])* $name ),*
        }

        impl SyntaxKind {
            /// Every kind, indexed by its `u16` value.
            const ALL: &'static [SyntaxKind] = &[$( SyntaxKind::$name ),*];
        }
    };
}

syntax_kinds! {
    // ---- trivia ----
    /// Spaces, tabs, carriage returns, and newlines the lexer dropped.
    WHITESPACE,
    /// `// ...` up to the end of the line.
    LINE_COMMENT,
    /// `/* ... */`
    BLOCK_COMMENT,

    // ---- significant tokens ----
    /// A newline that ends a field, an item or an entry.
    NEWLINE,
    /// `/// ...`: one line of a doc comment.
    DOC,
    /// A word that is not a keyword.
    IDENT,
    /// `19mm`, `3/4in`, `1-1/2in`, `18.5mm`, `110deg`
    QUANTITY,
    /// A unitless whole number: `4`.
    INT_NUMBER,
    /// A unitless decimal number: `0.5`.
    DECIMAL_NUMBER,
    /// The `"` that opens or closes a string.
    STRING_QUOTE,
    /// Literal text inside a string, escapes included as written.
    STRING_TEXT,
    /// The `{` that starts an interpolation inside a string.
    INTERP_OPEN,
    /// The `}` that ends an interpolation inside a string.
    INTERP_CLOSE,
    /// A character that starts no token.
    ERROR_TOKEN,
    /// The end of the input. Only used for lookahead; never in a tree.
    EOF,

    /// `{`
    L_BRACE,
    /// `}`
    R_BRACE,
    /// `(`
    L_PAREN,
    /// `)`
    R_PAREN,
    /// `[`
    L_BRACK,
    /// `]`
    R_BRACK,
    /// `,`
    COMMA,
    /// `:`
    COLON,
    /// `.`
    DOT,
    /// `..`
    DOT2,
    /// `=`
    EQ,
    /// `==`
    EQ2,
    /// `!=`
    NEQ,
    /// `<`
    LT,
    /// `<=`
    LE,
    /// `>`
    GT,
    /// `>=`
    GE,
    /// `+`
    PLUS,
    /// `-`
    MINUS,
    /// `*`
    STAR,
    /// `/`
    SLASH,
    /// `&&`
    AMP2,
    /// `||`
    PIPE2,
    /// `!`
    BANG,

    // ---- keywords ----
    /// `stave`
    KW_STAVE,
    /// `project`
    KW_PROJECT,
    /// `library`
    KW_LIBRARY,
    /// `import`
    KW_IMPORT,
    /// `private`
    KW_PRIVATE,
    /// `kind`
    KW_KIND,
    /// `recipe`
    KW_RECIPE,
    /// `param`
    KW_PARAM,
    /// `let`
    KW_LET,
    /// `region`
    KW_REGION,
    /// `stock`
    KW_STOCK,
    /// `part`
    KW_PART,
    /// `joint`
    KW_JOINT,
    /// `hardware`
    KW_HARDWARE,
    /// `assembly`
    KW_ASSEMBLY,
    /// `profile`
    KW_PROFILE,
    /// `component`
    KW_COMPONENT,
    /// `scene`
    KW_SCENE,
    /// `require`
    KW_REQUIRE,
    /// `patch`
    KW_PATCH,
    /// `add`
    KW_ADD,
    /// `set`
    KW_SET,
    /// `remove`
    KW_REMOVE,
    /// `place`
    KW_PLACE,
    /// `pose`
    KW_POSE,
    /// `at`
    KW_AT,
    /// `within`
    KW_WITHIN,
    /// `locked`
    KW_LOCKED,
    /// `fit`
    KW_FIT,
    /// `true`
    KW_TRUE,
    /// `false`
    KW_FALSE,

    // ---- nodes: files ----
    /// A design file: header, imports and items.
    SOURCE_FILE,
    /// A patch file: header and patches.
    PATCH_FILE,
    /// `stave 1`
    HEADER,
    /// `import "./casework" as cw`
    IMPORT,

    // ---- nodes: items ----
    /// `project "Title" { ... }`
    PROJECT,
    /// `library "Title" { ... }`
    LIBRARY,
    /// `kind name : base { ... }`
    KIND,
    /// `recipe name : operation { ... }`
    RECIPE,
    /// `stock name { ... }`
    STOCK,
    /// `profile name { ... }`
    PROFILE,
    /// `component Name { ... }`
    COMPONENT,
    /// `scene name "Title" { ... }`
    SCENE,
    /// `param name : Type = default within domain locked`
    PARAM,
    /// `let name = expr`
    LET,
    /// `region name[labels] = expr`
    REGION,
    /// `part name : stock { ... }` or `part name = mirror(...)`
    PART,
    /// `joint name : kind(...)`
    JOINT,
    /// `hardware name : kind(...) { place { ... } }`
    HARDWARE,
    /// `assembly name : Component { ... }` or `assembly name = mirror(...)`
    ASSEMBLY,
    /// `require name : condition "message"`
    REQUIRE,
    /// `patch "Summary" { ... }`
    PATCH,

    // ---- nodes: blocks and entries ----
    /// `{ entries }`, the one block form of the grammar.
    BLOCK,
    /// `name: expr`
    FIELD,
    /// `place { terms }`
    PLACE,
    /// `roles { name: Type ... }`
    ROLES,
    /// `name: Type` inside `roles { }`
    ROLE,
    /// `cut selector { operations }` in a kind
    CUT,
    /// `makes part name : stock { ... }` in a kind
    MAKES,
    /// `member name` or `member name { ... }` in a hardware kind
    MEMBER,
    /// `shape name: call` in a member's body
    SHAPE,
    /// `mechanism { entries }` in a hardware kind
    MECHANISM,
    /// `mount { fields }` in a hardware kind or a member's body
    MOUNT,
    /// `pivot a, b at point(...)` in a mechanism
    PIVOT,
    /// `slot a, b from point(...) to point(...)` in a mechanism
    SLOT,
    /// `rail a, b along word` in a mechanism
    RAIL,
    /// `couple a to b: ratio` in a mechanism
    COUPLE,
    /// `stop: value "message"` in a mechanism
    STOP,
    /// `mark { steps }` or `cut { steps }` in a recipe
    STEP_BLOCK,
    /// `at station: value` in a profile
    STATION,
    /// `pose target = value` in a scene
    POSE,
    /// A call whose head is fixed by its position: `flush(base.left)`, `ground`.
    CALL,

    // ---- nodes: patch operations ----
    /// `add item [to ref] [in "file"]`, `add import "..." [in "file"]`
    ADD_OP,
    /// `replace item`
    REPLACE_OP,
    /// `set path = expr`
    SET_OP,
    /// `unset path`
    UNSET_OP,
    /// `remove tag path`, `remove import "..." [in "file"]`
    REMOVE_OP,
    /// `rename tag path to name`
    RENAME_OP,

    // ---- nodes: names, references and arguments ----
    /// A word used as a name: declared names, members, field keys, roles.
    NAME,
    /// `[3]` or `[left, right]` after a declared name.
    ARRAY_SPEC,
    /// `oak_19`, `casework.Drawer`
    REF,
    /// `side_l.place.deep`, `drawer[top]`
    PATH,
    /// `[*]`, `[same]`, `[top]`, `[0]`
    INDEX,
    /// `rabbet(back, into: side_l)`: a kind and its arguments.
    KIND_CALL,
    /// `mirror(source, about: plane)`
    MIRROR,
    /// `( args )`
    ARG_LIST,
    /// `name: expr` or `expr` inside an argument list.
    ARG,
    /// `lo..hi` or `[a, b]` after `within`.
    DOMAIN,

    // ---- nodes: expressions ----
    /// A quantity, number, `true`, `false` or `fit`.
    LITERAL,
    /// A string: quotes, text and interpolations.
    STRING,
    /// `{name.member}` inside a string.
    INTERP,
    /// A bare name in an expression.
    NAME_REF,
    /// `base.top`
    MEMBER_EXPR,
    /// `shelf[0]`
    INDEX_EXPR,
    /// `between(a, b)`
    CALL_EXPR,
    /// `-x`, `!x`
    UNARY_EXPR,
    /// `a + b`
    BINARY_EXPR,
    /// `[a, b]`
    LIST_EXPR,
    /// `( expr )`
    PAREN_EXPR,
    /// `lo..hi` as an argument of a member's shape
    RANGE_EXPR,

    /// Source the parser could not make sense of, kept so no byte is lost.
    ERROR,
}

impl SyntaxKind {
    /// `true` for whitespace and comments, which carry no meaning.
    pub fn is_trivia(self) -> bool {
        matches!(
            self,
            SyntaxKind::WHITESPACE | SyntaxKind::LINE_COMMENT | SyntaxKind::BLOCK_COMMENT
        )
    }

    /// `true` for a keyword token.
    pub fn is_keyword(self) -> bool {
        (SyntaxKind::KW_STAVE..=SyntaxKind::KW_FALSE).contains(&self)
    }

    /// `true` for a word token: an identifier or a keyword.
    pub fn is_word(self) -> bool {
        self == SyntaxKind::IDENT || self.is_keyword()
    }

    /// `true` for the keywords that start a declaration (grammar, "Unclosed
    /// brackets"): an item keyword at the start of a line is a recovery point.
    pub fn is_item_keyword(self) -> bool {
        matches!(
            self,
            SyntaxKind::KW_IMPORT
                | SyntaxKind::KW_PROJECT
                | SyntaxKind::KW_LIBRARY
                | SyntaxKind::KW_PRIVATE
                | SyntaxKind::KW_KIND
                | SyntaxKind::KW_RECIPE
                | SyntaxKind::KW_STOCK
                | SyntaxKind::KW_PROFILE
                | SyntaxKind::KW_COMPONENT
                | SyntaxKind::KW_SCENE
                | SyntaxKind::KW_PARAM
                | SyntaxKind::KW_LET
                | SyntaxKind::KW_REGION
                | SyntaxKind::KW_PART
                | SyntaxKind::KW_JOINT
                | SyntaxKind::KW_HARDWARE
                | SyntaxKind::KW_ASSEMBLY
                | SyntaxKind::KW_REQUIRE
                | SyntaxKind::KW_PATCH
        )
    }

    /// `true` for the keywords that start an item allowed inside a
    /// `component` body (the grammar's `design_item`).
    pub fn is_design_item_keyword(self) -> bool {
        matches!(
            self,
            SyntaxKind::KW_PARAM
                | SyntaxKind::KW_LET
                | SyntaxKind::KW_REGION
                | SyntaxKind::KW_PART
                | SyntaxKind::KW_JOINT
                | SyntaxKind::KW_HARDWARE
                | SyntaxKind::KW_ASSEMBLY
                | SyntaxKind::KW_REQUIRE
        )
    }

    /// `true` for nodes that are expressions.
    pub fn is_expr(self) -> bool {
        matches!(
            self,
            SyntaxKind::LITERAL
                | SyntaxKind::STRING
                | SyntaxKind::NAME_REF
                | SyntaxKind::MEMBER_EXPR
                | SyntaxKind::INDEX_EXPR
                | SyntaxKind::CALL_EXPR
                | SyntaxKind::UNARY_EXPR
                | SyntaxKind::BINARY_EXPR
                | SyntaxKind::LIST_EXPR
                | SyntaxKind::PAREN_EXPR
                | SyntaxKind::RANGE_EXPR
        )
    }

    /// `true` for nodes that are items (top level, inside a component or a
    /// kind, or after a patch's `add` and `replace`).
    pub fn is_item(self) -> bool {
        matches!(
            self,
            SyntaxKind::PROJECT
                | SyntaxKind::LIBRARY
                | SyntaxKind::KIND
                | SyntaxKind::RECIPE
                | SyntaxKind::STOCK
                | SyntaxKind::PROFILE
                | SyntaxKind::COMPONENT
                | SyntaxKind::SCENE
                | SyntaxKind::PARAM
                | SyntaxKind::LET
                | SyntaxKind::REGION
                | SyntaxKind::PART
                | SyntaxKind::JOINT
                | SyntaxKind::HARDWARE
                | SyntaxKind::ASSEMBLY
                | SyntaxKind::REQUIRE
        )
    }

    /// The keyword kind for a word, if the word is a keyword.
    pub fn keyword(word: &str) -> Option<SyntaxKind> {
        Some(match word {
            "stave" => SyntaxKind::KW_STAVE,
            "project" => SyntaxKind::KW_PROJECT,
            "library" => SyntaxKind::KW_LIBRARY,
            "import" => SyntaxKind::KW_IMPORT,
            "private" => SyntaxKind::KW_PRIVATE,
            "kind" => SyntaxKind::KW_KIND,
            "recipe" => SyntaxKind::KW_RECIPE,
            "param" => SyntaxKind::KW_PARAM,
            "let" => SyntaxKind::KW_LET,
            "region" => SyntaxKind::KW_REGION,
            "stock" => SyntaxKind::KW_STOCK,
            "part" => SyntaxKind::KW_PART,
            "joint" => SyntaxKind::KW_JOINT,
            "hardware" => SyntaxKind::KW_HARDWARE,
            "assembly" => SyntaxKind::KW_ASSEMBLY,
            "profile" => SyntaxKind::KW_PROFILE,
            "component" => SyntaxKind::KW_COMPONENT,
            "scene" => SyntaxKind::KW_SCENE,
            "require" => SyntaxKind::KW_REQUIRE,
            "patch" => SyntaxKind::KW_PATCH,
            "add" => SyntaxKind::KW_ADD,
            "set" => SyntaxKind::KW_SET,
            "remove" => SyntaxKind::KW_REMOVE,
            "place" => SyntaxKind::KW_PLACE,
            "pose" => SyntaxKind::KW_POSE,
            "at" => SyntaxKind::KW_AT,
            "within" => SyntaxKind::KW_WITHIN,
            "locked" => SyntaxKind::KW_LOCKED,
            "fit" => SyntaxKind::KW_FIT,
            "true" => SyntaxKind::KW_TRUE,
            "false" => SyntaxKind::KW_FALSE,
            _ => return None,
        })
    }

    /// The fixed text of a punctuation token, used in "expected ..." messages.
    pub fn punctuation(self) -> Option<&'static str> {
        Some(match self {
            SyntaxKind::L_BRACE => "{",
            SyntaxKind::R_BRACE => "}",
            SyntaxKind::L_PAREN => "(",
            SyntaxKind::R_PAREN => ")",
            SyntaxKind::L_BRACK => "[",
            SyntaxKind::R_BRACK => "]",
            SyntaxKind::COMMA => ",",
            SyntaxKind::COLON => ":",
            SyntaxKind::DOT => ".",
            SyntaxKind::DOT2 => "..",
            SyntaxKind::EQ => "=",
            SyntaxKind::EQ2 => "==",
            SyntaxKind::NEQ => "!=",
            SyntaxKind::LT => "<",
            SyntaxKind::LE => "<=",
            SyntaxKind::GT => ">",
            SyntaxKind::GE => ">=",
            SyntaxKind::PLUS => "+",
            SyntaxKind::MINUS => "-",
            SyntaxKind::STAR => "*",
            SyntaxKind::SLASH => "/",
            SyntaxKind::AMP2 => "&&",
            SyntaxKind::PIPE2 => "||",
            SyntaxKind::BANG => "!",
            _ => return None,
        })
    }

    /// `true` if a newline directly after this token is dropped (grammar,
    /// "Newlines", rule 2). See [`crate::LINE_CONTINUATION_TOKENS`].
    pub fn continues_line(self) -> bool {
        matches!(
            self,
            SyntaxKind::COMMA
                | SyntaxKind::L_PAREN
                | SyntaxKind::L_BRACK
                | SyntaxKind::PLUS
                | SyntaxKind::MINUS
                | SyntaxKind::STAR
                | SyntaxKind::SLASH
                | SyntaxKind::EQ2
                | SyntaxKind::NEQ
                | SyntaxKind::LT
                | SyntaxKind::LE
                | SyntaxKind::GT
                | SyntaxKind::GE
                | SyntaxKind::AMP2
                | SyntaxKind::PIPE2
                | SyntaxKind::BANG
                | SyntaxKind::DOT2
                | SyntaxKind::KW_WITHIN
                | SyntaxKind::DOC
        )
    }
}

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        // A `#[repr(u16)]` fieldless enum's discriminant: lossless.
        Self(kind as u16)
    }
}

/// The `.stave` language, for [`rowan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StaveLanguage {}

impl Language for StaveLanguage {
    type Kind = SyntaxKind;

    fn kind_from_raw(raw: rowan::SyntaxKind) -> SyntaxKind {
        SyntaxKind::ALL
            .get(usize::from(raw.0))
            .copied()
            .unwrap_or(SyntaxKind::ERROR)
    }

    fn kind_to_raw(kind: SyntaxKind) -> rowan::SyntaxKind {
        kind.into()
    }
}

/// An immutable, shareable tree. Turn it into a [`SyntaxNode`] with
/// [`SyntaxNode::new_root`] to walk it.
pub use rowan::GreenNode;

/// A node of the concrete syntax tree, with its position and parent.
pub type SyntaxNode = rowan::SyntaxNode<StaveLanguage>;
/// A token of the concrete syntax tree, trivia included.
pub type SyntaxToken = rowan::SyntaxToken<StaveLanguage>;
/// A node or a token.
pub type SyntaxElement = rowan::NodeOrToken<SyntaxNode, SyntaxToken>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KEYWORDS, LINE_CONTINUATION_TOKENS};

    #[test]
    fn raw_kinds_round_trip() {
        for (index, kind) in SyntaxKind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, index);
            assert_eq!(StaveLanguage::kind_from_raw((*kind).into()), *kind);
        }
        assert_eq!(
            StaveLanguage::kind_from_raw(rowan::SyntaxKind(u16::MAX)),
            SyntaxKind::ERROR
        );
    }

    #[test]
    fn keyword_kinds_match_the_keyword_list() {
        for word in KEYWORDS {
            let kind = SyntaxKind::keyword(word).expect(word);
            assert!(kind.is_keyword(), "{word}");
        }
        let keyword_kinds = SyntaxKind::ALL.iter().filter(|kind| kind.is_keyword());
        assert_eq!(keyword_kinds.count(), KEYWORDS.len());
        assert_eq!(SyntaxKind::keyword("top"), None);
        assert_eq!(SyntaxKind::keyword("as"), None);
    }

    #[test]
    fn continuation_kinds_match_the_grammar_list() {
        let mut listed = Vec::new();
        for kind in SyntaxKind::ALL {
            if kind.continues_line() {
                let text = match kind {
                    SyntaxKind::DOC => "DOC",
                    SyntaxKind::KW_WITHIN => "within",
                    other => other.punctuation().expect("punctuation"),
                };
                listed.push(text);
            }
        }
        listed.sort_unstable();
        let mut expected = LINE_CONTINUATION_TOKENS.to_vec();
        expected.sort_unstable();
        assert_eq!(listed, expected);
    }

    #[test]
    fn item_keywords_match_the_grammar_list() {
        const GRAMMAR: &str = include_str!("../../../docs/grammar.md");
        let start = "Item keywords are the words that start a declaration: ";
        let from = GRAMMAR.find(start).expect("item keyword sentence") + start.len();
        let rest = &GRAMMAR[from..];
        let sentence = &rest[..rest.find(".\n").expect("end of sentence")];
        let listed: Vec<&str> = sentence
            .split(',')
            .map(|word| word.trim().trim_matches('`'))
            .collect();
        let mut from_kinds: Vec<&str> = KEYWORDS
            .iter()
            .copied()
            .filter(|word| SyntaxKind::keyword(word).is_some_and(SyntaxKind::is_item_keyword))
            .collect();
        let mut listed_sorted = listed.clone();
        listed_sorted.sort_unstable();
        from_kinds.sort_unstable();
        assert_eq!(listed_sorted, from_kinds);
    }
}
