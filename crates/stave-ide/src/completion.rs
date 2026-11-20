//! Completion and signature help: the words that fit where the cursor is
//! (grammar, "Tooling and editors", "What an editor gets").
//!
//! The position decides the vocabulary: placement terms at the start of a
//! line in `place { }`, face selectors after `cut` and cut operations
//! inside its block, base kinds after `kind name :`, the operation after
//! `recipe name :`, the declaration's fields in a field position, the
//! expected type's words in an argument or a field's value, a module's
//! public items after `module.`, labels inside `[ ]`, and units after a
//! number.
//!
//! Half-typed text is the normal case, so the position is read from the
//! tokens before the cursor rather than from the parsed tree: the lossless
//! tree keeps every token, even of a line the parser could not make sense
//! of. Unclosed brackets are closed at the next line that starts with an
//! item keyword, as the parser does (grammar, "Unclosed brackets"). Names
//! are resolved through the index, as go-to-definition resolves them.
//! What the words are comes from `stave-schema`'s tables, the ones the
//! checker reads ([`stave_schema::positions`]).

use std::collections::BTreeSet;

use stave_lang::ast::{BaseKind, CutOperation, Span, TypeName, Unit};
use stave_lang::placement::Face;
use stave_lang::{FileId, SyntaxKind, SyntaxNode, SyntaxToken};
use stave_schema::fields::{self, FieldSpec, Special, ValueType};
use stave_schema::kinds;
use stave_schema::operations::{self, ArgumentDefault, Selector};
use stave_schema::placement::{MadePartTerm, PlacementTerm};
use stave_schema::positions::{Argument, ArgumentPosition, Block, CallSite, Callee};
use stave_schema::recipes::{CuttingStep, MarkingStep};
use stave_schema::words::MotionWord;

use crate::analysis::{Analysis, FilePosition};
use crate::index::{FileData, Symbol, SymbolId, SymbolKind};

/// What a completion is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CompletionKind {
    /// A placement term in `place { }`: `flush`.
    PlacementTerm,
    /// A face selector after `cut`: `crossing`.
    Selector,
    /// A cut operation inside a cut's block: `notch`.
    CutOperation,
    /// The operation a recipe is for, after `recipe name :`: `notch`.
    Operation,
    /// A marking or cutting step inside a recipe's `mark { }` or `cut { }`.
    Step,
    /// A base kind after `kind name :`: `joint`.
    BaseKind,
    /// A field of the block the cursor is in: `length`.
    Field,
    /// A word of the expected type's closed list: `across`, `plywood`.
    Word,
    /// A unit after a number: `mm`.
    Unit,
    /// An item: a module's public item after `module.`, a component's
    /// param in an assembly.
    Item(SymbolKind),
    /// A label of an array inside `[ ]`.
    Label,
    /// A member after `.`: a face or a readable field of a part.
    Member,
}

/// One word completion offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The word, as it is written.
    pub label: String,
    /// What it is.
    pub kind: CompletionKind,
    /// A short description: the type a field expects, a term's arguments,
    /// an item's declaration.
    pub detail: String,
    /// Its doc comment, for an item that has one.
    pub docs: Option<String>,
    /// The text the word replaces: the part of a word already typed (and
    /// the rest of it, when the cursor is inside one), or an empty span at
    /// the cursor.
    pub replace: Span,
}

/// What a parameter of a call is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ParameterKind {
    /// A role of a kind: `piece`, `into`.
    Role,
    /// A param of a kind or a component: `depth`.
    Param,
    /// A setting every joint kind takes without declaring it: `glue`.
    Setting,
    /// An argument of a call fixed by the language: a placement term, a
    /// built-in function, a face selector or a cut operation.
    Argument,
}

/// One parameter in signature help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureParameter {
    /// Its name.
    pub name: String,
    /// What it is.
    pub kind: ParameterKind,
    /// `true` if it may be written without its name, in order.
    pub positional: bool,
    /// The type it expects, as written: `Parts`, `Length`, `an axis`.
    pub ty: Option<String>,
    /// What it is when left out, as written: `into.thickness / 3`.
    pub default: Option<String>,
    /// `true` if it must be given.
    pub required: bool,
    /// Its doc comment.
    pub docs: Option<String>,
}

/// The parameters of the kind, component or fixed call the cursor is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    /// The call on one line: `dado(piece, into, depth = into.thickness / 3,
    /// glue:, methods:)`.
    pub label: String,
    /// What is called: `dado`, `casework.FramePanelDoor`, `flush`.
    pub callee: String,
    /// Its parameters, positional ones first.
    pub parameters: Vec<SignatureParameter>,
    /// The parameter the cursor's argument fills, when it is known.
    pub active: Option<usize>,
    /// The callee's doc comment.
    pub docs: Option<String>,
}

impl Analysis {
    /// The words that fit at a position, in the order they are listed
    /// for that position (a table's order, or by name for items), those
    /// starting with what is already typed. Empty where nothing fits or
    /// the cursor is in a comment or a string.
    pub fn completions(&self, position: FilePosition) -> Vec<Completion> {
        self.kept_completions(position, || self.work_out_completions(position))
    }

    fn work_out_completions(&self, position: FilePosition) -> Vec<Completion> {
        let Some(root) = self.index.files.get(&position.file).map(FileData::root) else {
            return Vec::new();
        };
        let Some(cursor) = Cursor::new(&root, position.offset) else {
            return Vec::new();
        };
        let context = Context {
            analysis: self,
            file: position.file,
            node: node_at(&root, position.offset),
            cursor: &cursor,
        };
        let mut seen = BTreeSet::new();
        context
            .candidates()
            .into_iter()
            .filter(|candidate| candidate.label.starts_with(cursor.prefix.as_str()))
            .filter(|candidate| seen.insert(candidate.label.clone()))
            .map(|candidate| Completion {
                label: candidate.label,
                kind: candidate.kind,
                detail: candidate.detail,
                docs: candidate.docs,
                replace: cursor.replace,
            })
            .collect()
    }

    /// The parameters of the kind or component being called at a
    /// position (its roles and params, with defaults), or of the
    /// placement term, built-in function, face selector or cut operation.
    pub fn signature_help(&self, position: FilePosition) -> Option<SignatureHelp> {
        let root = self.index.files.get(&position.file).map(FileData::root)?;
        let cursor = Cursor::new(&root, position.offset)?;
        let context = Context {
            analysis: self,
            file: position.file,
            node: node_at(&root, position.offset),
            cursor: &cursor,
        };
        context.signature()
    }
}

// ---------------------------------------------------------------------------
// The tokens around the cursor
// ---------------------------------------------------------------------------

/// The significant tokens of a file and where the cursor stands in them.
struct Cursor {
    /// Every significant token of the file (no whitespace or comments).
    tokens: Vec<SyntaxToken>,
    /// How many tokens come before the word being typed.
    before: usize,
    /// The part of the word typed before the cursor.
    prefix: String,
    /// What a completion replaces.
    replace: Span,
    /// `true` right after a number: units are offered.
    after_number: bool,
    /// The brackets open at the cursor, outermost first, as indices into
    /// `tokens`.
    open: Vec<usize>,
}

fn token_start(token: &SyntaxToken) -> u32 {
    u32::from(token.text_range().start())
}

fn token_end(token: &SyntaxToken) -> u32 {
    u32::from(token.text_range().end())
}

fn is_trivia(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::WHITESPACE
            | SyntaxKind::LINE_COMMENT
            | SyntaxKind::BLOCK_COMMENT
            | SyntaxKind::DOC
            | SyntaxKind::EOF
    )
}

fn is_open(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::L_BRACE | SyntaxKind::L_PAREN | SyntaxKind::L_BRACK
    )
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
)]
fn closer_of(kind: SyntaxKind) -> Option<SyntaxKind> {
    match kind {
        SyntaxKind::L_BRACE => Some(SyntaxKind::R_BRACE),
        SyntaxKind::L_PAREN => Some(SyntaxKind::R_PAREN),
        SyntaxKind::L_BRACK => Some(SyntaxKind::R_BRACK),
        _ => None,
    }
}

fn is_close(kind: SyntaxKind) -> bool {
    matches!(
        kind,
        SyntaxKind::R_BRACE | SyntaxKind::R_PAREN | SyntaxKind::R_BRACK
    )
}

/// `true` for a token that separates the entries of a block or the
/// arguments of a call.
fn is_separator(kind: SyntaxKind) -> bool {
    matches!(kind, SyntaxKind::NEWLINE | SyntaxKind::COMMA)
}

/// The deepest node at an offset, for resolving names in their scope.
fn node_at(root: &SyntaxNode, offset: u32) -> SyntaxNode {
    let length = u32::from(root.text_range().end());
    if offset > length {
        return root.clone();
    }
    root.token_at_offset(rowan::TextSize::from(offset))
        .left_biased()
        .and_then(|token| token.parent())
        .unwrap_or_else(|| root.clone())
}

impl Cursor {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
    )]
    fn new(root: &SyntaxNode, offset: u32) -> Option<Cursor> {
        if offset > u32::from(root.text_range().end()) {
            return None;
        }
        let mut tokens = Vec::new();
        let mut touching: Option<SyntaxToken> = None;
        // Where the cursor's line starts.
        let mut line_start = 0u32;
        for token in root
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
        {
            if token_start(&token) < offset && offset <= token_end(&token) {
                touching = Some(token.clone());
            }
            if token_end(&token) <= offset
                && let Some(newline) = token.text().rfind('\n')
                && let Ok(newline) = u32::try_from(newline)
            {
                line_start = token_start(&token)
                    .saturating_add(newline)
                    .saturating_add(1);
            }
            if !is_trivia(token.kind()) {
                tokens.push(token);
            }
        }
        let mut prefix = String::new();
        let mut replace = Span::new(offset, offset);
        let mut after_number = false;
        let mut word: Option<SyntaxToken> = None;
        if let Some(token) = &touching {
            let start = token_start(token);
            let typed = usize::try_from(offset.checked_sub(start)?).ok()?;
            let text = token.text();
            match token.kind() {
                SyntaxKind::LINE_COMMENT | SyntaxKind::DOC => return None,
                SyntaxKind::BLOCK_COMMENT if offset < token_end(token) => return None,
                SyntaxKind::STRING_TEXT => return None,
                SyntaxKind::STRING_QUOTE if opens_string(token) => return None,
                SyntaxKind::QUANTITY => {
                    let before = text.get(..typed)?;
                    let magnitude = before.trim_end_matches(|c: char| c.is_ascii_alphabetic());
                    prefix = before.get(magnitude.len()..)?.to_owned();
                    let typed_unit = u32::try_from(prefix.len()).ok()?;
                    replace = Span::new(offset.checked_sub(typed_unit)?, token_end(token));
                    after_number = true;
                }
                SyntaxKind::INT_NUMBER | SyntaxKind::DECIMAL_NUMBER
                    if offset == token_end(token) =>
                {
                    after_number = true;
                }
                kind if kind.is_word() => {
                    prefix = text.get(..typed)?.to_owned();
                    replace = Span::new(start, token_end(token));
                    word = Some(token.clone());
                }
                _ => {}
            }
        }
        let before = match &word {
            Some(word) => tokens.iter().position(|t| t == word)?,
            // A closer of no width at the cursor is after it.
            None => tokens.partition_point(|t| token_end(t) <= offset && token_start(t) < offset),
        };
        let mut before = if after_number {
            // The number itself is not context.
            tokens.partition_point(|t| token_end(t) < offset)
        } else {
            before
        };
        // The lexer closes a bracket left open at the end of the line it
        // was most likely forgotten on, with a closer of no width; while
        // the cursor is still on that line, the bracket is open.
        while let Some(last) = before.checked_sub(1)
            && tokens.get(last).is_some_and(|t| {
                t.text_range().is_empty() && is_close(t.kind()) && token_start(t) >= line_start
            })
        {
            before = last;
        }
        let open = open_brackets(tokens.get(..before)?);
        Some(Cursor {
            tokens,
            before,
            prefix,
            replace,
            after_number,
            open,
        })
    }

    /// The tokens before the word being typed.
    fn context(&self) -> &[SyntaxToken] {
        self.tokens.get(..self.before).unwrap_or(&[])
    }

    fn token(&self, index: usize) -> Option<&SyntaxToken> {
        self.tokens.get(index)
    }

    fn text(&self, index: usize) -> &str {
        self.token(index).map_or("", SyntaxToken::text)
    }

    fn kind(&self, index: usize) -> Option<SyntaxKind> {
        self.token(index).map(SyntaxToken::kind)
    }

    /// The innermost open bracket, if any.
    fn innermost(&self) -> Option<usize> {
        self.open.last().copied()
    }

    /// The innermost open brace below the bracket at `depth` in
    /// [`Cursor::open`] (exclusive), if any.
    fn brace_below(&self, depth: usize) -> Option<(usize, usize)> {
        self.open
            .get(..depth)?
            .iter()
            .enumerate()
            .rev()
            .find(|(_, index)| self.kind(**index) == Some(SyntaxKind::L_BRACE))
            .map(|(depth, index)| (depth, *index))
    }

    /// The entries written so far after an open bracket (or from the start
    /// of the file), split at separators at the bracket's own depth; the
    /// last is the one the cursor is in.
    fn entries(&self, open: Option<usize>) -> Vec<&[SyntaxToken]> {
        let from = open.map_or(0, |index| index.saturating_add(1));
        let tokens = self.tokens.get(from..self.before).unwrap_or(&[]);
        let mut entries = Vec::new();
        let mut open: Vec<(SyntaxKind, ())> = Vec::new();
        let mut start = 0usize;
        for (index, token) in tokens.iter().enumerate() {
            let kind = token.kind();
            if open.is_empty() && is_separator(kind) {
                entries.push(tokens.get(start..index).unwrap_or(&[]));
                start = index.saturating_add(1);
            }
            track(&mut open, kind, ());
        }
        entries.push(tokens.get(start..).unwrap_or(&[]));
        entries
    }

    /// The entry the cursor is in, up to the word being typed.
    fn entry(&self, open: Option<usize>) -> &[SyntaxToken] {
        self.entries(open).last().copied().unwrap_or(&[])
    }

    /// The header of a block: the tokens on the line of its `{` before it,
    /// without `private`, `locked` or `solved`.
    fn head(&self, brace: usize) -> &[SyntaxToken] {
        let mut start = brace;
        while let Some(previous) = start.checked_sub(1) {
            if self.kind(previous) == Some(SyntaxKind::NEWLINE) {
                break;
            }
            start = previous;
        }
        let head = self.tokens.get(start..brace).unwrap_or(&[]);
        strip_prefixes(head)
    }

    /// The names written as fields directly in a block, wherever the
    /// cursor is: so a field already written is not offered again.
    fn written_fields(&self, brace: usize) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        let mut open: Vec<(SyntaxKind, ())> = Vec::new();
        let mut at_start = false;
        for (index, token) in self.tokens.iter().enumerate().skip(brace) {
            let kind = token.kind();
            let direct = open.len() == 1;
            if direct && is_separator(kind) {
                at_start = true;
            } else if direct
                && at_start
                && kind.is_word()
                && self.kind(index.saturating_add(1)) == Some(SyntaxKind::COLON)
                && index != self.before
            {
                names.insert(token.text().to_owned());
                at_start = false;
            } else if direct {
                at_start = false;
            }
            track(&mut open, kind, ());
            if open.is_empty() {
                break;
            }
            if index == brace {
                at_start = true;
            }
        }
        names
    }

    /// The argument the cursor is in, in a call or list opened at `open`:
    /// its place among the positional arguments, or its name. `None` when
    /// the cursor is past the start of an argument's value.
    fn argument(&self, open: usize, whole: bool) -> Option<ArgumentAt> {
        let entries = self.entries(Some(open));
        let (current, previous) = entries.split_last()?;
        let named = |entry: &[SyntaxToken]| matches!(entry, [name, colon, ..] if name.kind().is_word() && colon.kind() == SyntaxKind::COLON);
        if let [name, colon, rest @ ..] = current
            && name.kind().is_word()
            && colon.kind() == SyntaxKind::COLON
        {
            return (whole || rest.is_empty()).then(|| ArgumentAt::Named(name.text().to_owned()));
        }
        if !whole && !current.is_empty() {
            return None;
        }
        // A positional argument after a named one is a syntax error: it
        // fills nothing.
        if previous.iter().any(|entry| named(entry)) {
            return None;
        }
        let index = previous.iter().filter(|entry| !named(entry)).count();
        Some(ArgumentAt::Positional(index))
    }

    /// The words of a name written before `end` (inclusive): `casework`,
    /// `hw.brass_knob_26`, `drawer[top].front` (indices are skipped: the
    /// members of an element are the array's).
    fn chain(&self, end: usize) -> Vec<String> {
        let mut words = Vec::new();
        let mut index = end;
        loop {
            // Skip an index: `[top]`.
            while self.kind(index) == Some(SyntaxKind::R_BRACK) {
                let mut depth = 0usize;
                loop {
                    match self.kind(index) {
                        Some(SyntaxKind::R_BRACK) => depth = depth.saturating_add(1),
                        Some(SyntaxKind::L_BRACK) => depth = depth.saturating_sub(1),
                        Some(_) => {}
                        None => return Vec::new(),
                    }
                    if depth == 0 {
                        break;
                    }
                    let Some(previous) = index.checked_sub(1) else {
                        return Vec::new();
                    };
                    index = previous;
                }
                let Some(previous) = index.checked_sub(1) else {
                    return Vec::new();
                };
                index = previous;
            }
            let Some(token) = self.token(index) else {
                return Vec::new();
            };
            if !token.kind().is_word() {
                return Vec::new();
            }
            words.push(token.text().to_owned());
            match index.checked_sub(1) {
                Some(dot) if self.kind(dot) == Some(SyntaxKind::DOT) => {
                    let Some(previous) = dot.checked_sub(1) else {
                        break;
                    };
                    index = previous;
                }
                _ => break,
            }
        }
        words.reverse();
        words
    }
}

/// `true` if a quote opens its string (the cursor after it is inside).
fn opens_string(quote: &SyntaxToken) -> bool {
    quote
        .parent()
        .and_then(|string| string.first_token())
        .is_some_and(|first| first == *quote)
}

/// A header without the prefixes an item may have.
fn strip_prefixes(tokens: &[SyntaxToken]) -> &[SyntaxToken] {
    let mut tokens = tokens;
    while let [first, rest @ ..] = tokens
        && matches!(
            first.kind(),
            SyntaxKind::KW_PRIVATE | SyntaxKind::KW_LOCKED | SyntaxKind::KW_SOLVED
        )
    {
        tokens = rest;
    }
    tokens
}

/// Opens or closes a bracket: a closer closes the innermost bracket of
/// its kind and every bracket opened inside it, as `}` closes the `(` and
/// `[` left open in its block. `open` holds what each open bracket is
/// (its kind and a value to keep with it).
fn track<T>(open: &mut Vec<(SyntaxKind, T)>, kind: SyntaxKind, value: T) {
    if is_open(kind) {
        open.push((kind, value));
    } else if is_close(kind)
        && let Some(matching) = open
            .iter()
            .rposition(|(candidate, _)| closer_of(*candidate) == Some(kind))
    {
        open.truncate(matching);
    }
}

/// The brackets left open after `tokens`, as indices. The lexer has
/// already closed the brackets a line starting with an item keyword, a
/// `}` or the end of the file closes (grammar, "Unclosed brackets"),
/// with closers of no width, so they are closed here too.
fn open_brackets(tokens: &[SyntaxToken]) -> Vec<usize> {
    let mut open: Vec<(SyntaxKind, usize)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        track(&mut open, token.kind(), index);
    }
    open.into_iter().map(|(_, index)| index).collect()
}

/// Where an argument stands, owning its name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ArgumentAt {
    Positional(usize),
    Named(String),
}

impl ArgumentAt {
    fn position(&self) -> ArgumentPosition<'_> {
        match self {
            ArgumentAt::Positional(index) => ArgumentPosition::Positional(*index),
            ArgumentAt::Named(name) => ArgumentPosition::Named(name),
        }
    }
}

// ---------------------------------------------------------------------------
// What fits
// ---------------------------------------------------------------------------

/// A completion before it is filtered and given its span.
struct Candidate {
    label: String,
    kind: CompletionKind,
    detail: String,
    docs: Option<String>,
}

impl Candidate {
    fn new(label: impl Into<String>, kind: CompletionKind, detail: impl Into<String>) -> Self {
        Candidate {
            label: label.into(),
            kind,
            detail: detail.into(),
            docs: None,
        }
    }

    fn symbol(symbol: &Symbol) -> Self {
        Candidate {
            label: symbol.name.clone(),
            kind: CompletionKind::Item(symbol.kind),
            detail: symbol.signature.clone(),
            docs: docs_of(symbol),
        }
    }
}

/// A symbol's doc comment as one paragraph.
fn docs_of(symbol: &Symbol) -> Option<String> {
    let docs = symbol.docs.join(" ");
    let docs = docs.split_whitespace().collect::<Vec<_>>().join(" ");
    (!docs.is_empty()).then_some(docs)
}

/// The cursor in a loaded file.
struct Context<'a> {
    analysis: &'a Analysis,
    file: FileId,
    node: SyntaxNode,
    cursor: &'a Cursor,
}

impl Context<'_> {
    fn candidates(&self) -> Vec<Candidate> {
        let cursor = self.cursor;
        let context = cursor.context();
        if cursor.after_number {
            return if self.number_takes_unit() {
                units()
            } else {
                Vec::new()
            };
        }
        if let Some(dot) = context.len().checked_sub(1)
            && cursor.kind(dot) == Some(SyntaxKind::DOT)
        {
            return dot
                .checked_sub(1)
                .map(|end| self.members(end))
                .unwrap_or_default();
        }
        match cursor.innermost() {
            Some(open) if cursor.kind(open) == Some(SyntaxKind::L_PAREN) => self.in_call(open),
            Some(open) if cursor.kind(open) == Some(SyntaxKind::L_BRACK) => self.in_brackets(open),
            open => self.in_block(open),
        }
    }

    /// `true` unless the number before the cursor is one that never takes
    /// a unit: the version in `stave 1`, or a count or index in `[ ]`
    /// after a name (`part shelf[3]`, `shelf[2]`).
    fn number_takes_unit(&self) -> bool {
        let cursor = self.cursor;
        let previous = cursor.before.checked_sub(1);
        if previous.and_then(|index| cursor.kind(index)) == Some(SyntaxKind::KW_STAVE) {
            return false;
        }
        match cursor.innermost() {
            Some(open) if cursor.kind(open) == Some(SyntaxKind::L_BRACK) => {
                let named = open
                    .checked_sub(1)
                    .and_then(|index| cursor.kind(index))
                    .is_some_and(|kind| kind.is_word() || kind == SyntaxKind::R_BRACK);
                !named
            }
            Some(_) | None => true,
        }
    }

    // ---- blocks ----

    fn in_block(&self, open: Option<usize>) -> Vec<Candidate> {
        let cursor = self.cursor;
        let entry = strip_prefixes(cursor.entry(open));
        // `kind name :` and `recipe name :`, wherever they are written.
        if let [keyword, name, colon] = entry
            && name.kind().is_word()
            && colon.kind() == SyntaxKind::COLON
        {
            if keyword.kind() == SyntaxKind::KW_KIND {
                return BaseKind::ALL
                    .iter()
                    .map(|base| Candidate::new(base.word(), CompletionKind::BaseKind, "base kind"))
                    .collect();
            }
            if keyword.kind() == SyntaxKind::KW_RECIPE {
                return stave_lang::ast::Operation::all()
                    .map(|operation| {
                        Candidate::new(operation.word(), CompletionKind::Operation, "operation")
                    })
                    .collect();
            }
        }
        let head = open.map(|brace| cursor.head(brace)).unwrap_or(&[]);
        let first = head.first().map(SyntaxToken::kind);
        let first_text = head.first().map_or("", SyntaxToken::text);
        // `cut ` or `cut name: ` in a kind's body: a face selector.
        if first == Some(SyntaxKind::KW_KIND) {
            let is_cut_head = match entry {
                [cut] => cut.text() == "cut",
                [cut, name, colon] => {
                    cut.text() == "cut"
                        && name.kind().is_word()
                        && colon.kind() == SyntaxKind::COLON
                }
                _ => false,
            };
            if is_cut_head {
                return Selector::ALL
                    .iter()
                    .map(|selector| {
                        Candidate::new(
                            selector.word(),
                            CompletionKind::Selector,
                            call_text(selector.word(), &Callee::Selector(*selector).arguments()),
                        )
                    })
                    .collect();
            }
        }
        // `pose door_l = ` in a scene: a motion word.
        if let [pose, .., equals] = entry
            && pose.kind() == SyntaxKind::KW_POSE
            && equals.kind() == SyntaxKind::EQ
        {
            return MotionWord::ALL
                .iter()
                .map(|word| Candidate::new(word.word(), CompletionKind::Word, "'open' or 'closed'"))
                .collect();
        }
        if entry.is_empty() {
            let Some(brace) = open else {
                return Vec::new();
            };
            if first == Some(SyntaxKind::KW_PLACE) {
                return self.placement_terms();
            }
            if first_text == "cut" && head.len() == 1 {
                return CuttingStep::ALL
                    .iter()
                    .map(|step| Candidate::new(step.word(), CompletionKind::Step, "cutting step"))
                    .collect();
            }
            if first_text == "mark" && head.len() == 1 {
                return MarkingStep::ALL
                    .iter()
                    .map(|step| Candidate::new(step.word(), CompletionKind::Step, "marking step"))
                    .collect();
            }
            if first_text == "cut" {
                return cut_operations(head);
            }
            let written = cursor.written_fields(brace);
            if let Some(block) = block_of(head) {
                return block
                    .fields()
                    .into_iter()
                    .filter(|spec| !written.contains(spec.name))
                    .map(field_candidate)
                    .collect();
            }
            if first == Some(SyntaxKind::KW_ASSEMBLY) {
                return self
                    .component_params(head)
                    .into_iter()
                    .filter(|symbol| !written.contains(&symbol.name))
                    .map(Candidate::symbol)
                    .collect();
            }
            return Vec::new();
        }
        // `name: ` in a block: the field's words.
        if let [name, colon] = entry
            && name.kind().is_word()
            && colon.kind() == SyntaxKind::COLON
            && let Some(spec) = block_of(head).and_then(|block| block.field(name.text()))
        {
            return words(spec.ty);
        }
        // `name: ` in an assembly: the words of the component's param.
        if let [name, colon] = entry
            && name.kind().is_word()
            && colon.kind() == SyntaxKind::COLON
            && first == Some(SyntaxKind::KW_ASSEMBLY)
        {
            return self
                .component_params(head)
                .into_iter()
                .find(|param| param.name == name.text())
                .and_then(|param| param_parameter(param, ParameterKind::Param).ty)
                .map(|ty| written_type_words(&ty))
                .unwrap_or_default();
        }
        Vec::new()
    }

    /// The placement terms a `place { }` takes: a made part's own, or
    /// those that apply to hardware, or all of them.
    fn placement_terms(&self) -> Vec<Candidate> {
        let cursor = self.cursor;
        let depth = cursor.open.len().saturating_sub(1);
        let owner = cursor
            .brace_below(depth)
            .map(|(_, brace)| cursor.head(brace))
            .unwrap_or(&[]);
        let owner_first = owner.first();
        let terms: Vec<PlacementTerm> = if owner_first.is_some_and(|t| t.text() == "makes") {
            MadePartTerm::ALL
                .iter()
                .filter_map(|term| PlacementTerm::from_word(term.word()))
                .collect()
        } else if owner_first.is_some_and(|t| t.kind() == SyntaxKind::KW_HARDWARE) {
            PlacementTerm::ALL
                .into_iter()
                .filter(|term| term.applies_to_hardware())
                .collect()
        } else {
            PlacementTerm::ALL.to_vec()
        };
        terms
            .into_iter()
            .map(|term| {
                Candidate::new(
                    term.word(),
                    CompletionKind::PlacementTerm,
                    call_text(term.word(), &Callee::Placement(term).arguments()),
                )
            })
            .collect()
    }

    /// The params of the component an assembly's header names.
    fn component_params(&self, head: &[SyntaxToken]) -> Vec<&Symbol> {
        let Some(colon) = head.iter().position(|t| t.kind() == SyntaxKind::COLON) else {
            return Vec::new();
        };
        let words: Vec<String> = head
            .get(colon.saturating_add(1)..)
            .unwrap_or(&[])
            .iter()
            .filter(|t| t.kind().is_word())
            .map(|t| t.text().to_owned())
            .collect();
        let Some(component) = self
            .resolve(&words)
            .filter(|id| self.symbol_kind(*id) == Some(SymbolKind::Component))
        else {
            return Vec::new();
        };
        self.inside(component, SymbolKind::Param)
    }

    // ---- calls ----

    /// Where a call below the bracket at `depth` is written: in
    /// `place { }`, on a `cut` line or in its block, or elsewhere.
    fn call_site(&self, depth: usize) -> CallSite {
        let cursor = self.cursor;
        let Some((_, brace)) = cursor.brace_below(depth) else {
            return CallSite::Expression;
        };
        let head = cursor.head(brace);
        if head
            .first()
            .is_some_and(|t| t.kind() == SyntaxKind::KW_PLACE)
        {
            return CallSite::Place;
        }
        let in_cut_block = head.first().is_some_and(|t| t.text() == "cut");
        let on_cut_line = cursor
            .entry(Some(brace))
            .first()
            .is_some_and(|t| t.text() == "cut");
        if in_cut_block || on_cut_line {
            CallSite::Cut
        } else {
            CallSite::Expression
        }
    }

    fn callee(&self, open: usize, depth: usize) -> Option<Callee> {
        let cursor = self.cursor;
        let head = open.checked_sub(1)?;
        let word = cursor.token(head).filter(|t| t.kind().is_word())?;
        // A dotted name is never a fixed call.
        if head
            .checked_sub(1)
            .is_some_and(|dot| cursor.kind(dot) == Some(SyntaxKind::DOT))
        {
            return None;
        }
        let callee = Callee::from_word(word.text(), self.call_site(depth))?;
        // A kind in scope with a built-in's name is called instead.
        if matches!(callee, Callee::Builtin(_))
            && self
                .resolve(&[word.text().to_owned()])
                .is_some_and(|id| self.symbol_kind(id) == Some(SymbolKind::Kind))
        {
            return None;
        }
        Some(callee)
    }

    fn in_call(&self, open: usize) -> Vec<Candidate> {
        let cursor = self.cursor;
        let depth = cursor.open.len().saturating_sub(1);
        let Some(argument) = cursor.argument(open, false) else {
            return Vec::new();
        };
        if let Some(callee) = self.callee(open, depth) {
            return callee
                .expects(argument.position())
                .map(words)
                .unwrap_or_default();
        }
        // A kind's call: the words of the param or setting named.
        let ArgumentAt::Named(name) = argument else {
            return Vec::new();
        };
        let Some((_, kind)) = self.called_kind(open) else {
            return Vec::new();
        };
        self.kind_parameters(kind)
            .into_iter()
            .find(|parameter| parameter.name == name)
            .and_then(|parameter| parameter.ty)
            .map(|ty| written_type_words(&ty))
            .unwrap_or_default()
    }

    /// The kind a call opened at `open` calls, and its name as written.
    fn called_kind(&self, open: usize) -> Option<(Vec<String>, &Symbol)> {
        let head = open.checked_sub(1)?;
        let words = self.cursor.chain(head);
        let kind = self
            .resolve(&words)
            .filter(|id| self.symbol_kind(*id) == Some(SymbolKind::Kind))?;
        Some((words, self.analysis.symbol(kind)?))
    }

    // ---- brackets ----

    fn in_brackets(&self, open: usize) -> Vec<Candidate> {
        let cursor = self.cursor;
        if !cursor.entry(Some(open)).is_empty() {
            return Vec::new();
        }
        let Some(before) = open.checked_sub(1) else {
            return Vec::new();
        };
        match cursor.kind(before) {
            Some(kind) if kind.is_word() || kind == SyntaxKind::R_BRACK => {
                if self.declares_array(open) {
                    return Vec::new();
                }
                let words = cursor.chain(before);
                let Some(array) = self.resolve(&words) else {
                    return Vec::new();
                };
                self.inside(array, SymbolKind::Label)
                    .into_iter()
                    .map(|label| Candidate {
                        label: label.name.clone(),
                        kind: CompletionKind::Label,
                        detail: label.signature.clone(),
                        docs: None,
                    })
                    .collect()
            }
            Some(SyntaxKind::COLON) => {
                // A list as a field's value: its elements' words.
                let depth = cursor.open.len().saturating_sub(1);
                let Some((_, brace)) = cursor.brace_below(depth) else {
                    return Vec::new();
                };
                let name = before.checked_sub(1).map_or("", |index| cursor.text(index));
                block_of(cursor.head(brace))
                    .and_then(|block| block.field(name))
                    .map(|spec| words(spec.ty))
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// `true` if the `[` at `open` follows the name in a declaration:
    /// `part shelf[`, `assembly drawer[`.
    fn declares_array(&self, open: usize) -> bool {
        let cursor = self.cursor;
        let depth = cursor.open.len().saturating_sub(1);
        let outer = cursor.open.get(..depth).and_then(|o| o.last().copied());
        let entry = cursor.entries(outer);
        let Some(line) = entry.last() else {
            return false;
        };
        let line = strip_prefixes(line);
        matches!(
            line,
            [keyword, name, bracket, ..]
                if keyword.kind().is_item_keyword()
                    && name.kind().is_word()
                    && cursor.token(open) == Some(bracket)
        )
    }

    // ---- members ----

    fn members(&self, end: usize) -> Vec<Candidate> {
        let words = self.cursor.chain(end);
        let Some(base) = self.resolve(&words) else {
            return Vec::new();
        };
        let Some(symbol) = self.analysis.symbol(base) else {
            return Vec::new();
        };
        let index = &self.analysis.index;
        match symbol.kind {
            SymbolKind::Module => {
                let from = index.files.get(&self.file).and_then(|f| f.module);
                let mut items: Vec<&Symbol> = index
                    .symbols()
                    .filter(|s| s.module == symbol.module && s.container.is_none())
                    .filter(|s| !s.private || Some(s.module) == from)
                    .filter(|s| {
                        !matches!(
                            s.kind,
                            SymbolKind::Module | SymbolKind::Label | SymbolKind::Require
                        )
                    })
                    .collect();
                items.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
                items.into_iter().map(Candidate::symbol).collect()
            }
            SymbolKind::Part | SymbolKind::MadePart => {
                let mut members = faces();
                members.extend(readable(fields::PART));
                members.extend(fields::PART_EXTRA_MEMBERS.iter().map(|name| {
                    Candidate::new(*name, CompletionKind::Member, "member of every part")
                }));
                members
            }
            SymbolKind::Region => faces(),
            SymbolKind::Stock => readable(fields::STOCK),
            SymbolKind::Assembly => {
                let Some(component) = index.instance_of.get(&base) else {
                    return Vec::new();
                };
                index
                    .symbols()
                    .filter(|s| s.container == Some(*component) && s.kind != SymbolKind::Label)
                    .map(Candidate::symbol)
                    .collect()
            }
            SymbolKind::Component
            | SymbolKind::Kind
            | SymbolKind::Recipe
            | SymbolKind::Profile
            | SymbolKind::Scene
            | SymbolKind::Param
            | SymbolKind::Let
            | SymbolKind::Joint
            | SymbolKind::Hardware
            | SymbolKind::Require
            | SymbolKind::Role
            | SymbolKind::Member
            | SymbolKind::Label => Vec::new(),
        }
    }

    // ---- names ----

    /// The symbol a dotted name means here.
    fn resolve(&self, words: &[String]) -> Option<SymbolId> {
        let (first, rest) = words.split_first()?;
        let index = &self.analysis.index;
        let mut current = index.lookup(self.file, &self.node, first)?;
        for word in rest {
            current = index.member(self.file, current, word)?;
        }
        Some(current)
    }

    fn symbol_kind(&self, id: SymbolId) -> Option<SymbolKind> {
        self.analysis.symbol(id).map(|s| s.kind)
    }

    /// The symbols of one kind declared inside another, in order.
    fn inside(&self, container: SymbolId, kind: SymbolKind) -> Vec<&Symbol> {
        self.analysis
            .index
            .symbols()
            .filter(|s| s.container == Some(container) && s.kind == kind)
            .collect()
    }

    // ---- signature help ----

    fn signature(&self) -> Option<SignatureHelp> {
        let cursor = self.cursor;
        // The innermost call inside the innermost block.
        let call = cursor
            .open
            .iter()
            .enumerate()
            .rev()
            .take_while(|(_, index)| cursor.kind(**index) != Some(SyntaxKind::L_BRACE))
            .find(|(_, index)| cursor.kind(**index) == Some(SyntaxKind::L_PAREN))
            .map(|(depth, index)| (depth, *index));
        if let Some((depth, open)) = call {
            let argument = cursor.argument(open, true);
            if let Some(callee) = self.callee(open, depth) {
                let parameters: Vec<SignatureParameter> =
                    callee.arguments().iter().map(argument_parameter).collect();
                return Some(help(callee.word().to_owned(), parameters, argument, None));
            }
            let (words, symbol) = self.called_kind(open)?;
            let parameters = self.kind_parameters(symbol);
            return Some(help(words.join("."), parameters, argument, docs_of(symbol)));
        }
        // A field of an assembly sets its component's params.
        let brace = cursor.innermost()?;
        let head = cursor.head(brace);
        if head.first().map(SyntaxToken::kind) != Some(SyntaxKind::KW_ASSEMBLY) {
            return None;
        }
        let colon = head.iter().position(|t| t.kind() == SyntaxKind::COLON)?;
        let words: Vec<String> = head
            .get(colon.saturating_add(1)..)?
            .iter()
            .filter(|t| t.kind().is_word())
            .map(|t| t.text().to_owned())
            .collect();
        let component = self
            .resolve(&words)
            .filter(|id| self.symbol_kind(*id) == Some(SymbolKind::Component))?;
        let symbol = self.analysis.symbol(component)?;
        let parameters = self
            .inside(component, SymbolKind::Param)
            .into_iter()
            .map(|param| param_parameter(param, ParameterKind::Param))
            .collect();
        let argument = match cursor.entry(Some(brace)) {
            [name, colon, ..] if name.kind().is_word() && colon.kind() == SyntaxKind::COLON => {
                Some(ArgumentAt::Named(name.text().to_owned()))
            }
            _ => None,
        };
        Some(help(words.join("."), parameters, argument, docs_of(symbol)))
    }

    /// A kind's roles, then its params, then the settings every joint
    /// kind takes.
    fn kind_parameters(&self, kind: &Symbol) -> Vec<SignatureParameter> {
        let base = kind
            .signature
            .rsplit(':')
            .next()
            .and_then(|base| BaseKind::from_word(base.trim()));
        let mut parameters: Vec<SignatureParameter> = self
            .inside(kind.id, SymbolKind::Role)
            .into_iter()
            .map(|role| SignatureParameter {
                name: role.name.clone(),
                kind: ParameterKind::Role,
                positional: true,
                ty: role
                    .signature
                    .split_once(':')
                    .map(|(_, ty)| ty.trim().to_owned()),
                default: None,
                required: true,
                docs: docs_of(role),
            })
            .collect();
        if parameters.is_empty() && base == Some(BaseKind::Joint) {
            parameters.extend(
                kinds::JOINT_DEFAULT_ROLES
                    .iter()
                    .map(|role| SignatureParameter {
                        name: role.name.to_owned(),
                        kind: ParameterKind::Role,
                        positional: true,
                        ty: Some(role_type_text(role.ty).to_owned()),
                        default: None,
                        required: true,
                        docs: None,
                    }),
            );
        }
        parameters.extend(
            self.inside(kind.id, SymbolKind::Param)
                .into_iter()
                .map(|param| param_parameter(param, ParameterKind::Param)),
        );
        if base == Some(BaseKind::Joint) {
            parameters.extend(fields::JOINT_IMPLICIT_SETTINGS.iter().map(|spec| {
                SignatureParameter {
                    name: spec.name.to_owned(),
                    kind: ParameterKind::Setting,
                    positional: false,
                    ty: Some(type_text(spec.ty)),
                    default: None,
                    required: false,
                    docs: None,
                }
            }));
        }
        parameters
    }
}

// ---------------------------------------------------------------------------
// Vocabularies
// ---------------------------------------------------------------------------

/// Every unit suffix, in the order the grammar lists them.
fn units() -> Vec<Candidate> {
    Unit::ALL
        .iter()
        .map(|unit| Candidate::new(unit.suffix(), CompletionKind::Unit, unit_name(*unit)))
        .collect()
}

fn unit_name(unit: Unit) -> &'static str {
    match unit {
        Unit::Millimeter => "millimetres",
        Unit::Centimeter => "centimetres",
        Unit::Meter => "metres",
        Unit::Inch => "inches",
        Unit::Foot => "feet",
        Unit::Degree => "degrees",
    }
}

/// The six faces, as members.
fn faces() -> Vec<Candidate> {
    Face::ALL
        .iter()
        .map(|face| Candidate::new(face.word(), CompletionKind::Member, "face"))
        .collect()
}

/// The readable fields of a table, as members.
fn readable(table: &'static [FieldSpec]) -> Vec<Candidate> {
    table
        .iter()
        .filter(|spec| spec.readable)
        .map(|spec| Candidate::new(spec.name, CompletionKind::Member, type_text(spec.ty)))
        .collect()
}

/// The words a type may be written as.
fn words(ty: ValueType) -> Vec<Candidate> {
    let detail = type_text(ty);
    ty.words()
        .into_iter()
        .map(|word| Candidate::new(word, CompletionKind::Word, detail.clone()))
        .collect()
}

/// The words of a type written in a declaration (`Bool`, `Direction`);
/// none for a type that is not one of the language's names.
fn written_type_words(ty: &str) -> Vec<Candidate> {
    TypeName::from_word(ty.trim())
        .map(|name| words(ValueType::Type(name)))
        .unwrap_or_default()
}

fn field_candidate(spec: &FieldSpec) -> Candidate {
    Candidate::new(spec.name, CompletionKind::Field, type_text(spec.ty))
}

/// The block whose fields a header's block takes.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "SyntaxKind is a flat set of about 140 token and node tags: this tests the few it needs and treats the rest alike"
)]
fn block_of(head: &[SyntaxToken]) -> Option<Block> {
    let first = head.first()?;
    Some(match first.kind() {
        SyntaxKind::KW_PROJECT => Block::Project,
        SyntaxKind::KW_LIBRARY => Block::Library,
        SyntaxKind::KW_STOCK => Block::Stock,
        SyntaxKind::KW_PART => Block::Part,
        SyntaxKind::KW_PROFILE => Block::Profile,
        SyntaxKind::KW_SCENE => Block::Scene,
        SyntaxKind::KW_RECIPE => Block::Recipe,
        SyntaxKind::KW_KIND => Block::Kind(BaseKind::from_word(head.get(3)?.text())?),
        SyntaxKind::IDENT if first.text() == "makes" => Block::Part,
        _ => return None,
    })
}

/// The cut operations a cut's block takes: those that may be cut on its
/// face selector (every one that is cut on some face when the header
/// names none; `chamfer` and `roundover` are written only in `edges:`).
fn cut_operations(head: &[SyntaxToken]) -> Vec<Candidate> {
    let selector = head.iter().find_map(|t| Selector::from_word(t.text()));
    CutOperation::ALL
        .into_iter()
        .filter(|operation| {
            let selectors = operations::cut_operation(*operation).selectors;
            match selector {
                Some(selector) => selectors.contains(&selector),
                None => !selectors.is_empty(),
            }
        })
        .map(|operation| {
            Candidate::new(
                operation.word(),
                CompletionKind::CutOperation,
                call_text(operation.word(), &Callee::Cut(operation).arguments()),
            )
        })
        .collect()
}

/// A fixed call written with its arguments: `against(face, gap:)`.
fn call_text(word: &str, arguments: &[Argument]) -> String {
    if arguments.is_empty() {
        return word.to_owned();
    }
    let arguments: Vec<String> = arguments
        .iter()
        .map(|argument| {
            if argument.positional {
                argument.name.to_owned()
            } else {
                format!("{}:", argument.name)
            }
        })
        .collect();
    format!("{word}({})", arguments.join(", "))
}

/// A type in words: `Length`, `an axis`, `list of Tool`.
fn type_text(ty: ValueType) -> String {
    match ty {
        ValueType::Type(name) => name.word().to_owned(),
        ValueType::Words(set) => set.description().to_owned(),
        ValueType::LengthOrFit => "Length or fit".to_owned(),
        ValueType::Depth => "Length or through".to_owned(),
        ValueType::ListOf(inner) => format!("list of {}", type_text(*inner)),
        ValueType::Items => "items".to_owned(),
        ValueType::Special(special) => match special {
            Special::InsertSteps => "assembly steps",
            Special::CutFirst => "roles",
            Special::Adds => "extend(...)",
            Special::Uses => "fastener kinds",
            Special::Needs => "needs",
            Special::Edges => "edge profiles",
            Special::Banding => "edge banding",
        }
        .to_owned(),
    }
}

fn role_type_text(ty: kinds::RoleType) -> &'static str {
    match ty {
        kinds::RoleType::Part => "Part",
        kinds::RoleType::Parts => "Parts",
        kinds::RoleType::Region => "Region",
    }
}

/// A default from a table, as written.
fn default_text(default: ArgumentDefault) -> Option<String> {
    match default {
        ArgumentDefault::Required | ArgumentDefault::Absent => None,
        ArgumentDefault::Length(nanometres) => {
            Some(stave_lang::units::Length::from_nm(nanometres).to_string())
        }
        ArgumentDefault::Angle(microdegrees) => {
            Some(stave_lang::units::Angle::from_microdeg(microdegrees).to_string())
        }
        ArgumentDefault::Count(count) => Some(count.to_string()),
        ArgumentDefault::Bool(value) => Some(value.to_string()),
    }
}

fn argument_parameter(argument: &Argument) -> SignatureParameter {
    SignatureParameter {
        name: argument.name.to_owned(),
        kind: ParameterKind::Argument,
        positional: argument.positional,
        ty: Some(type_text(argument.ty)),
        default: default_text(argument.default),
        required: argument.default == ArgumentDefault::Required,
        docs: None,
    }
}

/// A param as a parameter: its type and default read off its
/// declaration (`param depth : Length = 9mm within ...`).
fn param_parameter(param: &Symbol, kind: ParameterKind) -> SignatureParameter {
    let declaration = param
        .signature
        .split_once("param ")
        .map_or("", |(_, rest)| rest);
    let declaration = declaration.split(" within ").next().unwrap_or(declaration);
    let (left, default) = match declaration.split_once('=') {
        Some((left, default)) => (left, Some(default.trim().to_owned())),
        None => (declaration, None),
    };
    let ty = left.split_once(':').map(|(_, ty)| ty.trim().to_owned());
    SignatureParameter {
        name: param.name.clone(),
        kind,
        positional: false,
        ty,
        required: default.is_none(),
        default,
        docs: docs_of(param),
    }
}

/// Signature help for a callee and the argument the cursor is in.
fn help(
    callee: String,
    parameters: Vec<SignatureParameter>,
    argument: Option<ArgumentAt>,
    docs: Option<String>,
) -> SignatureHelp {
    let active = argument.and_then(|argument| match argument {
        ArgumentAt::Named(name) => parameters.iter().position(|p| p.name == name),
        ArgumentAt::Positional(index) => parameters
            .iter()
            .enumerate()
            .filter(|(_, p)| p.positional)
            .nth(index)
            .map(|(position, _)| position),
    });
    let written: Vec<String> = parameters
        .iter()
        .map(
            |parameter| match (&parameter.default, parameter.positional) {
                (_, true) => parameter.name.clone(),
                (Some(default), false) => format!("{} = {default}", parameter.name),
                (None, false) => format!("{}:", parameter.name),
            },
        )
        .collect();
    SignatureHelp {
        label: format!("{callee}({})", written.join(", ")),
        callee,
        parameters,
        active,
        docs,
    }
}
