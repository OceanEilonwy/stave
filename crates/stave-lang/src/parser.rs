//! The parser: tokens to a lossless concrete syntax tree.
//!
//! It is a hand-written recursive descent parser with precedence climbing for
//! expressions. Hand-written, because error recovery is the hard part of a
//! parser that an editor runs on every keystroke, and the grammar's recovery
//! points (items start with a keyword on a new line, a field ends at its
//! line, blocks use braces) are easiest to honour with plain code. Two tokens
//! of lookahead decide every choice (grammar, "Notes for the parser").
//!
//! The parser never fails: it always builds a tree that holds every token,
//! wrapping what it could not understand in [`SyntaxKind::ERROR`] nodes, and
//! reports each mistake once:
//!
//! - an error inside an item or a block entry skips to the end of its line;
//! - a body that meets an item keyword it can't contain ends there with
//!   `error[missing-brace]`, and the item is parsed by the enclosing scope;
//! - an error inside `( )` or `[ ]` skips to the matching closer, which the
//!   lexer guarantees exists.

use rowan::{Checkpoint, GreenNode, GreenNodeBuilder};

use crate::ast::Span;
use crate::diagnostics::{Code, Diagnostic, FileId, Label};
use crate::lexer::Token;
use crate::syntax::SyntaxKind::{self, *};

/// Expressions nested deeper than this are rejected instead of risking the
/// stack.
const MAX_DEPTH: u32 = 128;

/// What the input is expected to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    /// A design file (`file` in the grammar).
    File,
    /// A patch file (`patch_file` in the grammar).
    PatchFile,
}

/// Marker for "an error was reported here"; the caller recovers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reported;

type PResult = Result<(), Reported>;

/// Parses tokens into a green tree.
pub(crate) fn parse(
    file: FileId,
    text: &str,
    tokens: &[Token],
    entry: Entry,
) -> (GreenNode, Vec<Diagnostic>) {
    let significant: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| !token.kind.is_trivia())
        .map(|(index, _)| index)
        .collect();
    let mut parser = Parser {
        file,
        text,
        tokens,
        significant,
        cursor: 0,
        emitted: 0,
        builder: GreenNodeBuilder::new(),
        diagnostics: Vec::new(),
        depth: 0,
        interpolation_allowed: false,
    };
    match entry {
        Entry::File => parser.file(),
        Entry::PatchFile => parser.patch_file(),
    }
    (parser.builder.finish(), parser.diagnostics)
}

/// Where an item is being parsed; decides what may follow and how to
/// recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// The top level of a file.
    TopLevel,
    /// Inside a `component` body.
    Component,
    /// Inside a `kind` body (only `param` and `require` are items there).
    Kind,
    /// After a patch's `add` or `replace`.
    Patch,
}

impl Scope {
    /// Recovery inside a block stops at its `}`; at the top level a stray `}`
    /// is junk.
    fn stop_at_brace(self) -> bool {
        self != Scope::TopLevel
    }
}

/// The vocabulary of a call head fixed by position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Placement,
    Selector,
    Operation,
    Step,
}

impl CallKind {
    /// The plural, for "... are calls" help lines.
    fn plural(self) -> &'static str {
        match self {
            CallKind::Placement => "placement terms",
            CallKind::Selector => "face selectors",
            CallKind::Operation => "cut operations",
            CallKind::Step => "steps",
        }
    }

    fn what(self) -> &'static str {
        match self {
            CallKind::Placement => "a placement term",
            CallKind::Selector => "a face selector",
            CallKind::Operation => "a cut operation",
            CallKind::Step => "a step",
        }
    }
}

/// What a `{ }` block holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    /// Fields only: project, library, stock, assembly.
    Fields,
    /// Fields and one `place`: part, made part.
    Part,
    /// One `place` only: hardware.
    Hardware,
    /// Kind entries.
    Kind,
    /// Recipe entries.
    Recipe,
    /// Profile stations and fields.
    Profile,
    /// Design items.
    Component,
    /// Poses and fields.
    Scene,
    /// `name: Type` roles.
    Roles,
    /// A member's shapes and fields.
    Member,
    /// A mechanism's entries.
    Mechanism,
    /// One call per entry.
    Calls(CallKind),
    /// Patch operations.
    Patch,
}

impl Body {
    /// `true` if an entry starting with this item keyword belongs in the
    /// body (so it does not trigger the missing-brace rule).
    fn allows_item(self, keyword: SyntaxKind) -> bool {
        match self {
            Body::Component => keyword.is_design_item_keyword(),
            Body::Kind => matches!(keyword, KW_PARAM | KW_REQUIRE),
            Body::Fields
            | Body::Part
            | Body::Hardware
            | Body::Recipe
            | Body::Profile
            | Body::Scene
            | Body::Roles
            | Body::Member
            | Body::Mechanism
            | Body::Calls(_)
            | Body::Patch => false,
        }
    }
}

struct Parser<'t> {
    file: FileId,
    text: &'t str,
    tokens: &'t [Token],
    /// Indexes into `tokens` of the significant (non-trivia) tokens.
    significant: Vec<usize>,
    /// Index into `significant` of the current token.
    cursor: usize,
    /// Number of tokens already added to the tree.
    emitted: usize,
    builder: GreenNodeBuilder<'static>,
    diagnostics: Vec<Diagnostic>,
    depth: u32,
    /// `true` while parsing the value of `note:` or `check:`, or a
    /// `require` message, where `{name}` is allowed in strings.
    interpolation_allowed: bool,
}

// ---------------------------------------------------------------------------
// Infrastructure
// ---------------------------------------------------------------------------

impl<'t> Parser<'t> {
    fn token(&self, n: usize) -> Option<&'t Token> {
        let tokens: &'t [Token] = self.tokens;
        self.significant
            .get(self.cursor + n)
            .and_then(|&index| tokens.get(index))
    }

    fn nth(&self, n: usize) -> SyntaxKind {
        self.token(n).map_or(EOF, |token| token.kind)
    }

    fn current(&self) -> SyntaxKind {
        self.nth(0)
    }

    fn at(&self, kind: SyntaxKind) -> bool {
        self.current() == kind
    }

    fn token_text(&self, token: &Token) -> &'t str {
        let text: &'t str = self.text;
        let start = usize::try_from(token.span.start()).unwrap_or(usize::MAX);
        let end = usize::try_from(token.span.end()).unwrap_or(usize::MAX);
        text.get(start..end).unwrap_or("")
    }

    fn nth_text(&self, n: usize) -> &'t str {
        self.token(n).map_or("", |token| self.token_text(token))
    }

    /// `true` if the current token is the identifier `word`.
    fn at_word(&self, word: &str) -> bool {
        self.at(IDENT) && self.nth_text(0) == word
    }

    /// `true` if the token at `n` is a word (identifier or keyword).
    fn nth_is_word(&self, n: usize) -> bool {
        self.nth(n).is_word()
    }

    fn end_offset(&self) -> u32 {
        u32::try_from(self.text.len()).unwrap_or(u32::MAX)
    }

    fn span(&self, n: usize) -> Span {
        self.token(n).map_or_else(
            || Span::new(self.end_offset(), self.end_offset()),
            |token| token.span,
        )
    }

    /// `true` if the current token is the first on its line.
    fn at_line_start(&self) -> bool {
        match self.cursor.checked_sub(1) {
            None => true,
            Some(previous) => self
                .significant
                .get(previous)
                .and_then(|&index| self.tokens.get(index))
                .is_some_and(|token| token.kind == NEWLINE),
        }
    }

    fn emit(&mut self, token: Token) {
        let text = self.token_text(&token);
        self.builder.token(token.kind.into(), text);
    }

    /// Adds the trivia before the current token to the tree.
    fn flush_trivia(&mut self) {
        let until = self
            .significant
            .get(self.cursor)
            .copied()
            .unwrap_or(self.tokens.len());
        while self.emitted < until {
            if let Some(&token) = self.tokens.get(self.emitted) {
                self.emit(token);
            }
            self.emitted += 1;
        }
    }

    /// Adds the current token (and the trivia before it) to the tree.
    fn bump(&mut self) {
        self.flush_trivia();
        if let Some(&token) = self.token(0) {
            self.emit(token);
            self.emitted += 1;
            self.cursor += 1;
        }
    }

    fn eat(&mut self, kind: SyntaxKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn start(&mut self, kind: SyntaxKind) {
        self.flush_trivia();
        self.builder.start_node(kind.into());
    }

    fn finish(&mut self) {
        self.builder.finish_node();
    }

    fn checkpoint(&mut self) -> Checkpoint {
        self.flush_trivia();
        self.builder.checkpoint()
    }

    /// Runs `body` inside a node of `kind`; the node is always finished.
    fn node(&mut self, kind: SyntaxKind, body: impl FnOnce(&mut Self) -> PResult) -> PResult {
        self.start(kind);
        let result = body(self);
        self.finish();
        result
    }

    /// Runs `body` inside a node of `kind` that starts at `checkpoint`.
    fn node_at(
        &mut self,
        checkpoint: Checkpoint,
        kind: SyntaxKind,
        body: impl FnOnce(&mut Self) -> PResult,
    ) -> PResult {
        self.builder.start_node_at(checkpoint, kind.into());
        let result = body(self);
        self.finish();
        result
    }

    /// Wraps the current token in a node of `kind`.
    fn wrap(&mut self, kind: SyntaxKind) {
        self.start(kind);
        self.bump();
        self.finish();
    }

    // ---- diagnostics ----

    /// How the token at `n` is described after "found".
    fn describe(&self, n: usize) -> String {
        match self.nth(n) {
            EOF => "end of file".to_owned(),
            NEWLINE => "end of line".to_owned(),
            DOC => "a doc comment".to_owned(),
            STRING_QUOTE => "a string".to_owned(),
            _ => {
                let text = self.nth_text(n);
                if text.chars().count() > 32 {
                    let short: String = text.chars().take(29).collect();
                    format!("'{short}...'")
                } else {
                    format!("'{text}'")
                }
            }
        }
    }

    fn line_of(&self, offset: u32) -> usize {
        let end = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.text.len());
        self.text
            .as_bytes()
            .get(..end)
            .map_or(0, |before| before.iter().filter(|&&b| b == b'\n').count())
            + 1
    }

    fn report(&mut self, diagnostic: Diagnostic) -> Reported {
        self.diagnostics.push(diagnostic);
        Reported
    }

    /// `expected <a> or <b>, found <current>`, at the current token.
    fn expected(&mut self, expectations: &[&str]) -> Reported {
        self.expected_with_help(expectations, None)
    }

    fn expected_with_help(&mut self, expectations: &[&str], help: Option<&str>) -> Reported {
        let message = format!(
            "expected {}, found {}",
            expectations.join(" or "),
            self.describe(0)
        );
        let mut diagnostic =
            Diagnostic::error(Code::Syntax, message, Label::new(self.file, self.span(0)));
        diagnostic.help = Some(
            help.map(str::to_owned)
                .or_else(|| self.operator_at_line_start_help())
                .unwrap_or_else(|| default_help(expectations)),
        );
        self.report(diagnostic)
    }

    /// Help for a line that starts with a binary operator: the expression
    /// on the line before ended at its newline.
    fn operator_at_line_start_help(&self) -> Option<String> {
        let binary = matches!(
            self.current(),
            PLUS | MINUS | STAR | SLASH | EQ2 | NEQ | LT | LE | GT | GE | AMP2 | PIPE2 | DOT2
        );
        (binary && self.at_line_start()).then(|| {
            format!(
                "a line can't start with an operator; end the previous line with '{}' to continue the expression",
                self.nth_text(0)
            )
        })
    }

    /// `true` if the current token can start an expression.
    fn at_expression_start(&self) -> bool {
        matches!(
            self.current(),
            IDENT
                | QUANTITY
                | INT_NUMBER
                | DECIMAL_NUMBER
                | STRING_QUOTE
                | L_BRACK
                | L_PAREN
                | MINUS
                | BANG
                | KW_TRUE
                | KW_FALSE
                | KW_FIT
        )
    }

    /// Help for a call head written without parentheses (`on base.top`):
    /// the previous token is the head and an expression follows on the same
    /// line. In a block of calls any head counts; in an expression only a
    /// built-in function does.
    fn missing_parentheses_help(&self, context: Option<CallKind>) -> Option<String> {
        if !self.at_expression_start() || self.at_line_start() {
            return None;
        }
        let previous = self
            .cursor
            .checked_sub(1)
            .and_then(|index| self.significant.get(index))
            .and_then(|&index| self.tokens.get(index))?;
        if previous.kind != IDENT {
            return None;
        }
        let head = self.token_text(previous);
        let what = match context {
            Some(call) => call.plural(),
            None if crate::BUILTIN_FUNCTIONS.contains(&head) => "built-in functions",
            None => return None,
        };
        // The rest of the line, when it is a simple path, goes in the
        // suggestion.
        let mut n = 0;
        let mut end = None;
        loop {
            let kind = self.nth(n);
            let previous = n.checked_sub(1).map(|p| self.nth(p));
            // A word may not follow a word or an index: `a b` is not a path.
            let word_after_word = kind.is_word()
                && previous.is_some_and(|previous| previous.is_word() || previous == R_BRACK);
            match kind {
                NEWLINE | EOF | R_BRACE | L_BRACE => break,
                _ if word_after_word => {
                    end = None;
                    break;
                }
                DOT | L_BRACK | R_BRACK | STAR | INT_NUMBER => {}
                kind if kind.is_word() => {}
                _ => {
                    end = None;
                    break;
                }
            }
            end = Some(self.span(n).end());
            n += 1;
        }
        let arguments = end
            .and_then(|end| {
                self.text
                    .get(usize::try_from(self.span(0).start()).ok()?..usize::try_from(end).ok()?)
            })
            .unwrap_or("...");
        Some(format!("{what} are calls: write {head}({arguments})"))
    }

    fn expect_with_help(&mut self, kind: SyntaxKind, what: &str, help: &str) -> PResult {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.expected_with_help(&[what], Some(help)))
        }
    }

    fn expect(&mut self, kind: SyntaxKind, what: &str) -> PResult {
        if self.eat(kind) {
            Ok(())
        } else {
            Err(self.expected(&[what]))
        }
    }

    /// `true` if the current token ends a value: the value is missing.
    fn at_value_end(&self) -> bool {
        matches!(
            self.current(),
            NEWLINE | EOF | R_BRACE | COMMA | R_PAREN | R_BRACK
        )
    }

    /// `'<name>' has no value`, pointing at `name` and its `:` or `=`.
    fn missing_value(&mut self, name: &str, at: Span) -> Reported {
        let diagnostic = Diagnostic::error(
            Code::MissingValue,
            format!("'{name}' has no value"),
            Label::new(self.file, at),
        )
        .with_help("write the value on the same line, or remove the line");
        self.report(diagnostic)
    }

    /// `<owner> is missing its closing "}"`.
    fn missing_brace(&mut self, owner: &str, open: Span) {
        let mut diagnostic = Diagnostic::error(
            Code::MissingBrace,
            format!("{owner} is missing its closing \"}}\""),
            Label::with_message(self.file, open, "this block is never closed"),
        );
        if self.at(EOF) {
            diagnostic = diagnostic.with_help("add \"}\" at the end of the file");
        } else {
            let line = self.line_of(self.span(0).start());
            diagnostic = diagnostic
                .with_note(format!("the next declaration starts on line {line}"))
                .with_help(format!("add \"}}\" before line {line}"));
        }
        self.diagnostics.push(diagnostic);
    }

    // ---- recovery ----

    /// Skips, inside an error node, to the end of the line. Braces are
    /// skipped as balanced groups; inside them an item keyword at the start
    /// of a line stops the skip.
    fn skip_rest(&mut self, stop_at_brace: bool) {
        let stops = |parser: &Self| match parser.current() {
            NEWLINE | EOF => true,
            R_BRACE => stop_at_brace,
            _ => false,
        };
        if stops(self) {
            return;
        }
        self.start(ERROR);
        let mut depth: u32 = 0;
        loop {
            match self.current() {
                EOF => break,
                NEWLINE if depth == 0 => break,
                R_BRACE if depth == 0 => {
                    if stop_at_brace {
                        break;
                    }
                    self.bump();
                }
                R_BRACE => {
                    depth = depth.saturating_sub(1);
                    self.bump();
                }
                L_BRACE => {
                    depth += 1;
                    self.bump();
                }
                kind if depth > 0
                    && kind.is_item_keyword()
                    && self.at_line_start()
                    && self.nth(1) != COLON =>
                {
                    break;
                }
                _ => self.bump(),
            }
        }
        self.finish();
    }

    /// Skips to the closer that matches an opener already consumed, and
    /// consumes it. The lexer guarantees every `(` and `[` has one.
    fn skip_to_closer(&mut self, closer: SyntaxKind) {
        let mut depth: u32 = 0;
        if self.at(closer) {
            self.bump();
            return;
        }
        self.start(ERROR);
        loop {
            match self.current() {
                EOF => break,
                kind if kind == closer && depth == 0 => break,
                L_PAREN | L_BRACK => {
                    depth += 1;
                    self.bump();
                }
                R_PAREN | R_BRACK => {
                    depth = depth.saturating_sub(1);
                    self.bump();
                }
                // A brace can't be inside brackets in a correct file, and the
                // lexer closes brackets before a `}`; stop rather than eat
                // the enclosing block.
                R_BRACE | NEWLINE if depth == 0 => break,
                _ => self.bump(),
            }
        }
        self.finish();
        self.eat(closer);
    }

    // -----------------------------------------------------------------------
    // Files
    // -----------------------------------------------------------------------

    fn skip_newlines(&mut self) {
        while self.at(NEWLINE) {
            self.bump();
        }
    }

    /// After an item at the top level: the line must end.
    fn end_of_line(&mut self, result: PResult, stop_at_brace: bool) {
        match self.current() {
            NEWLINE | EOF => {}
            R_BRACE if stop_at_brace => {}
            _ if self.at_line_start() => {}
            _ => {
                if result.is_ok() {
                    let help = self.missing_parentheses_help(None);
                    self.expected_with_help(&["end of line"], help.as_deref());
                }
                self.skip_rest(stop_at_brace);
            }
        }
    }

    fn file(&mut self) {
        // The root holds everything, leading trivia included.
        self.builder.start_node(SOURCE_FILE.into());
        self.skip_newlines();
        let header = self.header();
        self.end_of_line(header, false);
        let mut seen_item = false;
        loop {
            self.skip_newlines();
            let before = self.cursor;
            let result = match self.current() {
                EOF => break,
                KW_IMPORT => {
                    if seen_item {
                        self.expected_with_help(
                            &["a declaration"],
                            Some("imports go before every declaration"),
                        );
                    }
                    self.import()
                }
                KW_PATCH => Err(self
                    .expected_with_help(&["a declaration"], Some("a patch goes in a patch file"))),
                kind if kind == DOC || kind == KW_PRIVATE || kind.is_item_keyword() => {
                    seen_item = true;
                    self.item(Scope::TopLevel)
                }
                _ => Err(self.expected(&["a declaration"])),
            };
            if self.cursor == before {
                self.skip_rest(false);
                continue;
            }
            self.end_of_line(result, false);
        }
        self.flush_trivia();
        self.finish();
    }

    fn patch_file(&mut self) {
        // The root holds everything, leading trivia included.
        self.builder.start_node(PATCH_FILE.into());
        self.skip_newlines();
        let header = self.header();
        self.end_of_line(header, false);
        loop {
            self.skip_newlines();
            let before = self.cursor;
            let result = match self.current() {
                EOF => break,
                KW_PATCH => self.patch(),
                _ => Err(self.expected(&["'patch'"])),
            };
            if self.cursor == before {
                self.skip_rest(false);
                continue;
            }
            self.end_of_line(result, false);
        }
        self.flush_trivia();
        self.finish();
    }

    /// `stave 1`. A missing header is reported without consuming anything,
    /// so the first item still parses.
    fn header(&mut self) -> PResult {
        if !self.at(KW_STAVE) {
            // Reported without consuming anything: the line is left for the
            // item loop, which starts at the beginning of a line.
            self.expected_with_help(
                &["'stave'"],
                Some("every file starts with the line: stave 1"),
            );
            return Ok(());
        }
        self.node(HEADER, |p| {
            p.bump();
            if !p.at(INT_NUMBER) {
                return Err(p.expected_with_help(&["the format version"], Some("write: stave 1")));
            }
            let version_fits = crate::lexer::parse_number(p.nth_text(0))
                .ok()
                .and_then(|number| match number {
                    crate::ast::Number::Integer(value) => u32::try_from(value).ok(),
                    crate::ast::Number::Decimal(_) => None,
                })
                .is_some();
            if !version_fits {
                return Err(p.expected_with_help(&["the format version"], Some("write: stave 1")));
            }
            p.bump();
            Ok(())
        })
    }

    /// `import "./casework" as cw`
    fn import(&mut self) -> PResult {
        self.node(IMPORT, |p| {
            p.bump();
            if !p.at(STRING_QUOTE) {
                return Err(p.expected(&["the module's source in quotes"]));
            }
            p.string();
            if p.at_word("as") {
                p.bump();
                p.declared_name()?;
            }
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Items
    // -----------------------------------------------------------------------

    /// `{ DOC } [ "private" ] item_kind`
    fn item(&mut self, scope: Scope) -> PResult {
        let checkpoint = self.checkpoint();
        while self.at(DOC) {
            self.bump();
        }
        if self.at(KW_PRIVATE) {
            if matches!(scope, Scope::Component | Scope::Kind) {
                self.expected_with_help(
                    &["a declaration"],
                    Some("everything inside a component or kind is already private to it"),
                );
            }
            self.bump();
        }
        let kind = match self.current() {
            KW_PROJECT => PROJECT,
            KW_LIBRARY => LIBRARY,
            KW_KIND => KIND,
            KW_RECIPE => RECIPE,
            KW_STOCK => STOCK,
            KW_PROFILE => PROFILE,
            KW_COMPONENT => COMPONENT,
            KW_SCENE => SCENE,
            KW_PARAM => PARAM,
            KW_LET => LET,
            KW_REGION => REGION,
            KW_PART => PART,
            KW_JOINT => JOINT,
            KW_HARDWARE => HARDWARE,
            KW_ASSEMBLY => ASSEMBLY,
            KW_REQUIRE => REQUIRE,
            _ => {
                let reported = self.expected_with_help(
                    &["a declaration"],
                    Some("doc comments go right before a declaration; use // for other comments"),
                );
                return self.node_at(checkpoint, ERROR, |p| {
                    p.skip_rest(scope.stop_at_brace());
                    Err(reported)
                });
            }
        };
        self.node_at(checkpoint, kind, |p| {
            let result = match kind {
                PROJECT => p.project(),
                LIBRARY => p.library(),
                KIND => p.kind(),
                RECIPE => p.recipe(),
                STOCK => p.named_block_item("stock", Body::Fields),
                PROFILE => p.named_block_item("profile", Body::Profile),
                COMPONENT => p.named_block_item("component", Body::Component),
                SCENE => p.scene(),
                PARAM => p.param(),
                LET => p.binding(),
                REGION => p.region(),
                PART => p.part_or_assembly("part", Body::Part),
                JOINT => p.joint(),
                HARDWARE => p.hardware(),
                ASSEMBLY => p.part_or_assembly("assembly", Body::Fields),
                _ => p.require(),
            };
            // Whatever is left on the line belongs to this item, so that a
            // mistake there marks the item as broken. After a patch's `add`,
            // `to` and `in` may follow on the same line.
            let result = match result {
                Ok(()) if scope != Scope::Patch && !p.at_item_end(scope) => {
                    let help = p.missing_parentheses_help(None);
                    Err(p.expected_with_help(&["end of line"], help.as_deref()))
                }
                other => other,
            };
            if result.is_err() {
                p.skip_rest(scope.stop_at_brace());
            }
            result
        })
    }

    /// `true` if the current token can follow a complete item.
    fn at_item_end(&self, scope: Scope) -> bool {
        match self.current() {
            NEWLINE | EOF => true,
            R_BRACE | COMMA => scope.stop_at_brace(),
            _ => self.at_line_start(),
        }
    }

    /// The text of the name just parsed (the last token), for messages.
    fn previous_text(&self) -> &'t str {
        match self.cursor.checked_sub(1) {
            Some(previous) => self
                .significant
                .get(previous)
                .and_then(|&index| self.tokens.get(index))
                .map_or("", |token| self.token_text(token)),
            None => "",
        }
    }

    fn project(&mut self) -> PResult {
        self.bump();
        if !self.at(STRING_QUOTE) {
            return Err(self.expected(&["the project's title in quotes"]));
        }
        self.string();
        self.block(Body::Fields, "the project")
    }

    fn library(&mut self) -> PResult {
        self.bump();
        if !self.at(STRING_QUOTE) {
            return Err(self.expected(&["the library's title in quotes"]));
        }
        self.string();
        if self.at(L_BRACE) {
            self.block(Body::Fields, "the library")?;
        }
        Ok(())
    }

    /// `stock`, `profile` and `component`: keyword, name, block.
    fn named_block_item(&mut self, keyword: &str, body: Body) -> PResult {
        self.bump();
        self.declared_name()?;
        let owner = format!("{keyword} '{}'", self.previous_text());
        self.block(body, &owner)
    }

    fn kind(&mut self) -> PResult {
        self.bump();
        self.declared_name()?;
        let owner = format!("kind '{}'", self.previous_text());
        self.expect_with_help(COLON, "':'", "a kind is written: kind name : joint { ... }")?;
        let is_base = self.at(KW_JOINT)
            || (self.at(IDENT) && crate::ast::BaseKind::from_word(self.nth_text(0)).is_some());
        if !is_base {
            return Err(self.expected(&[
                "'joint'",
                "'slide'",
                "'hinge'",
                "'pull'",
                "'fastener'",
                "'tool'",
            ]));
        }
        self.wrap(NAME);
        self.block(Body::Kind, &owner)
    }

    fn recipe(&mut self) -> PResult {
        self.bump();
        self.declared_name()?;
        let owner = format!("recipe '{}'", self.previous_text());
        self.expect_with_help(
            COLON,
            "':'",
            "a recipe is written: recipe name : operation { ... }",
        )?;
        if !self.at(IDENT) {
            return Err(self.expected(&["an operation"]));
        }
        self.wrap(NAME);
        self.block(Body::Recipe, &owner)
    }

    fn scene(&mut self) -> PResult {
        self.bump();
        self.declared_name()?;
        let owner = format!("scene '{}'", self.previous_text());
        if self.at(STRING_QUOTE) {
            self.string();
        }
        self.block(Body::Scene, &owner)
    }

    /// `param name [: Type] [= default] [within domain] [locked]`
    fn param(&mut self) -> PResult {
        self.bump();
        let name_span = self.span(0);
        self.declared_name()?;
        let name = self.previous_text();
        if !self.at(COLON) && !self.at(EQ) {
            return Err(self.expected_with_help(
                &["':'", "'='"],
                Some("a param is written: param name = default, or param name : Type"),
            ));
        }
        if self.eat(COLON) {
            self.type_name()?;
        }
        if self.at(EQ) {
            let at = name_span.to(self.span(0));
            self.bump();
            self.value(name, at)?;
        }
        if self.eat(KW_WITHIN) {
            self.domain()?;
        }
        self.eat(KW_LOCKED);
        Ok(())
    }

    fn type_name(&mut self) -> PResult {
        if self.at(IDENT) {
            self.wrap(NAME);
            Ok(())
        } else {
            Err(self.expected(&["a type"]))
        }
    }

    /// `lo..hi` or `[a, b, c]`
    fn domain(&mut self) -> PResult {
        self.node(DOMAIN, |p| {
            if p.at(L_BRACK) {
                return p.list();
            }
            // Both ends are `add_expr`s: everything tighter than a comparison.
            p.expr_bp(BinaryPower::Compare)?;
            p.expect(DOT2, "'..'")?;
            p.expr_bp(BinaryPower::Compare)
        })
    }

    /// `let name = expr`
    fn binding(&mut self) -> PResult {
        self.bump();
        let name_span = self.span(0);
        self.declared_name()?;
        let name = self.previous_text();
        let at = name_span.to(self.span(0));
        self.expect_with_help(EQ, "'='", "a let is written: let name = value")?;
        self.value(name, at)
    }

    /// `region name [array] = expr`
    fn region(&mut self) -> PResult {
        self.bump();
        let name_span = self.span(0);
        self.declared_name()?;
        let name = self.previous_text();
        self.array_spec()?;
        let at = name_span.to(self.span(0));
        self.expect_with_help(EQ, "'='", "a region is written: region name = opening(...)")?;
        self.value(name, at)
    }

    /// `part name [array] (: ref block | = mirror(...))`, and the same for
    /// `assembly`.
    fn part_or_assembly(&mut self, keyword: &str, body: Body) -> PResult {
        self.bump();
        let name_span = self.span(0);
        self.declared_name()?;
        let name = self.previous_text();
        let owner = format!("{keyword} '{name}'");
        self.array_spec()?;
        match self.current() {
            COLON => {
                self.bump();
                self.reference()?;
                self.block(body, &owner)
            }
            EQ => {
                let at = name_span.to(self.span(0));
                self.bump();
                if self.at_value_end() {
                    return Err(self.missing_value(name, at));
                }
                self.mirror(keyword)
            }
            _ => {
                let source = if keyword == "part" {
                    "stock"
                } else {
                    "Component"
                };
                let help = format!(
                    "a {keyword} is written: {keyword} name : {source} {{ ... }}, or {keyword} name = mirror(...)"
                );
                Err(self.expected_with_help(&["':'", "'='"], Some(&help)))
            }
        }
    }

    /// `mirror(source, about: plane)`
    fn mirror(&mut self, keyword: &str) -> PResult {
        if !(self.at_word("mirror") && self.nth(1) == L_PAREN) {
            let help = format!(
                "a derived {keyword} is always a mirror image: mirror(side_l, about: mid(base, across))"
            );
            return Err(self.expected_with_help(&["'mirror'"], Some(&help)));
        }
        self.node(MIRROR, |p| {
            p.bump();
            p.bump();
            let inside = (|| {
                p.expr()?;
                p.expect(COMMA, "','")?;
                if !p.at_word("about") || p.nth(1) != COLON {
                    return Err(p.expected(&["'about:'"]));
                }
                p.wrap(NAME);
                p.bump();
                p.expr()?;
                p.eat(COMMA);
                if p.at(R_PAREN) {
                    Ok(())
                } else {
                    Err(p.expected(&["')'"]))
                }
            })();
            // Reported inside; skip what is left of the call either way.
            let _ = inside;
            p.skip_to_closer(R_PAREN);
            Ok(())
        })
    }

    /// `joint name [array] : kind(...)`; a joint has no body.
    fn joint(&mut self) -> PResult {
        self.bump();
        self.declared_name()?;
        self.array_spec()?;
        self.expect_with_help(
            COLON,
            "':'",
            "a joint is written: joint name : kind(parts, settings)",
        )?;
        self.kind_call()?;
        if self.at(L_BRACE) {
            return Err(self.expected_with_help(
                &["end of line"],
                Some("a joint's settings go inside the parentheses"),
            ));
        }
        Ok(())
    }

    /// `hardware name [array] : kind(...) [{ place { ... } }]`
    fn hardware(&mut self) -> PResult {
        self.bump();
        self.declared_name()?;
        let owner = format!("hardware '{}'", self.previous_text());
        self.array_spec()?;
        self.expect_with_help(
            COLON,
            "':'",
            "hardware is written: hardware name : kind(part, settings)",
        )?;
        self.kind_call()?;
        if self.at(L_BRACE) {
            self.block(Body::Hardware, &owner)?;
        }
        Ok(())
    }

    /// `require name : condition "message"`
    fn require(&mut self) -> PResult {
        self.bump();
        let name_span = self.span(0);
        self.declared_name()?;
        let name = self.previous_text();
        let at = name_span.to(self.span(0));
        self.expect_with_help(
            COLON,
            "':'",
            "a require is written: require name : condition \"message\"",
        )?;
        if self.at_value_end() {
            return Err(self.missing_value(name, at));
        }
        self.expr()?;
        if !self.at(STRING_QUOTE) {
            return Err(self.expected_with_help(
                &["a message in quotes"],
                Some("a require ends with the message shown when it fails"),
            ));
        }
        let allowed = std::mem::replace(&mut self.interpolation_allowed, true);
        self.string();
        self.interpolation_allowed = allowed;
        Ok(())
    }

    /// `patch "summary" { operations }`
    fn patch(&mut self) -> PResult {
        self.node(PATCH, |p| {
            p.bump();
            if !p.at(STRING_QUOTE) {
                let reported = p.expected(&["the patch's summary in quotes"]);
                p.skip_rest(false);
                return Err(reported);
            }
            let start = p.span(0).start();
            p.string();
            let summary = p
                .text
                .get(
                    usize::try_from(start).unwrap_or(0)
                        ..usize::try_from(p.span(0).start()).unwrap_or(0),
                )
                .unwrap_or("")
                .trim_end()
                .to_owned();
            let result = p.block(Body::Patch, &format!("patch {summary}"));
            if result.is_err() {
                p.skip_rest(false);
            }
            result
        })
    }

    // -----------------------------------------------------------------------
    // Names, references, arrays, calls
    // -----------------------------------------------------------------------

    /// A new name (`IDENT`). A keyword is reported as a reserved word and
    /// parsed as the name, so the rest of the declaration still parses.
    fn declared_name(&mut self) -> PResult {
        match self.current() {
            IDENT => {
                self.wrap(NAME);
                Ok(())
            }
            kind if kind.is_keyword() => {
                let word = self.nth_text(0);
                let diagnostic = Diagnostic::error(
                    Code::ReservedWord,
                    format!("'{word}' is a reserved word and can't be used as a name"),
                    Label::new(self.file, self.span(0)),
                )
                .with_help(format!("choose another name, such as 'my_{word}'"));
                self.diagnostics.push(diagnostic);
                self.wrap(NAME);
                Ok(())
            }
            _ => Err(self.expected(&["a name"])),
        }
    }

    /// Any word (`NAME`): after `.` and before `:`.
    fn any_name(&mut self) -> PResult {
        if self.nth_is_word(0) {
            self.wrap(NAME);
            Ok(())
        } else {
            Err(self.expected(&["a name"]))
        }
    }

    /// The root of a reference or path: an `IDENT`.
    fn root_name(&mut self) -> PResult {
        if self.at(IDENT) {
            self.wrap(NAME);
            Ok(())
        } else {
            Err(self.expected(&["a name"]))
        }
    }

    /// `ident { . name }`
    fn reference(&mut self) -> PResult {
        self.node(REF, |p| {
            p.root_name()?;
            while p.eat(DOT) {
                p.any_name()?;
            }
            Ok(())
        })
    }

    /// `ident { . name | [ index ] }`
    fn path(&mut self) -> PResult {
        self.node(PATH, |p| {
            p.root_name()?;
            loop {
                match p.current() {
                    DOT => {
                        p.bump();
                        p.any_name()?;
                    }
                    L_BRACK => p.index(),
                    _ => return Ok(()),
                }
            }
        })
    }

    /// `[3]` or `[left, right]` after a declared name, if present.
    fn array_spec(&mut self) -> PResult {
        if !self.at(L_BRACK) {
            return Ok(());
        }
        self.node(ARRAY_SPEC, |p| {
            p.bump();
            let labels = p.nth_is_word(0) && p.nth(1) == COMMA;
            let inside = if labels { p.labels() } else { p.expr() };
            if inside.is_ok() && !p.at(R_BRACK) {
                p.expected(&["']'"]);
            }
            p.skip_to_closer(R_BRACK);
            Ok(())
        })
    }

    /// `a, b, c` inside an array spec: two or more different labels.
    fn labels(&mut self) -> PResult {
        let mut seen: Vec<&str> = Vec::new();
        loop {
            let span = self.span(0);
            let word = self.nth_text(0);
            self.declared_name()?;
            if seen.contains(&word) {
                let diagnostic = Diagnostic::error(
                    Code::DuplicateName,
                    format!("'{word}' is declared twice"),
                    Label::new(self.file, span),
                )
                .with_help("every label of an array must be different");
                self.diagnostics.push(diagnostic);
            }
            seen.push(word);
            if !self.eat(COMMA) {
                break;
            }
            if self.at(R_BRACK) {
                break;
            }
        }
        if seen.len() < 2 {
            return Err(self.expected_with_help(
                &["a second label"],
                Some("a labelled array needs two or more labels; for one, just name the item"),
            ));
        }
        if self.at(R_BRACK) {
            Ok(())
        } else {
            Err(self.expected(&["','", "']'"]))
        }
    }

    /// `[ * | same | expr ]`
    fn index(&mut self) {
        let _ = self.node(INDEX, |p| {
            p.bump();
            // `[*]` and `[same]` are whole indexes; anything else is an
            // expression (a label or a number).
            let whole = (p.at(STAR) || p.at_word("same")) && p.nth(1) == R_BRACK;
            let inside = if whole {
                p.bump();
                Ok(())
            } else {
                p.expr()
            };
            if inside.is_ok() && !p.at(R_BRACK) {
                p.expected(&["']'"]);
            }
            p.skip_to_closer(R_BRACK);
            Ok(())
        });
    }

    /// `ref ( args )`
    fn kind_call(&mut self) -> PResult {
        self.node(KIND_CALL, |p| {
            p.reference()?;
            if !p.at(L_PAREN) {
                return Err(p.expected_with_help(
                    &["'('"],
                    Some("name the kind with its parts and settings: dado(shelf, into: side_l)"),
                ));
            }
            p.arg_list();
            Ok(())
        })
    }

    /// A call whose head is fixed by position: `flush(base.left)`, `ground`.
    fn call(&mut self, call: CallKind) -> PResult {
        if !self.at(IDENT) {
            return Err(self.expected(&[call.what()]));
        }
        self.node(CALL, |p| {
            p.wrap(NAME);
            if p.at(L_PAREN) {
                p.arg_list();
            }
            Ok(())
        })
    }

    /// `( [ arg { , arg } [ , ] ] )`
    fn arg_list(&mut self) {
        let _ = self.node(ARG_LIST, |p| {
            p.bump();
            let mut seen_named = false;
            let inside = (|| {
                while !p.at(R_PAREN) {
                    let named = p.nth_is_word(0) && p.nth(1) == COLON;
                    if seen_named && !named {
                        p.expected_with_help(
                            &["a named argument"],
                            Some("positional arguments come before named ones"),
                        );
                    }
                    seen_named |= named;
                    p.arg(named)?;
                    if !p.eat(COMMA) {
                        if p.at(R_PAREN) {
                            break;
                        }
                        return Err(p.expected(&["','", "')'"]));
                    }
                }
                Ok(())
            })();
            let _ = inside;
            p.skip_to_closer(R_PAREN);
            Ok(())
        });
    }

    /// `[ name : ] expr`
    fn arg(&mut self, named: bool) -> PResult {
        self.node(ARG, |p| {
            if named {
                let name = p.nth_text(0);
                let at = p.span(0).to(p.span(1));
                p.wrap(NAME);
                p.bump();
                if p.at_value_end() {
                    // `f(depth: )`: report, and carry on with the next one.
                    p.missing_value(name, at);
                    return Ok(());
                }
            }
            let checkpoint = p.checkpoint();
            p.expr()?;
            if p.at(DOT2) {
                p.node_at(checkpoint, RANGE_EXPR, |p| {
                    p.bump();
                    if p.at_value_end() {
                        return Err(p.expected(&["an expression"]));
                    }
                    p.expr()
                })?;
            }
            Ok(())
        })
    }

    // -----------------------------------------------------------------------
    // Blocks and entries
    // -----------------------------------------------------------------------

    /// `{ entries }`. `owner` names the block in a missing-brace message,
    /// e.g. "part 'side_l'".
    fn block(&mut self, body: Body, owner: &str) -> PResult {
        if !self.at(L_BRACE) {
            return Err(self.expected(&["'{'"]));
        }
        let description = match body {
            Body::Roles => format!("the roles block of {owner}"),
            Body::Calls(CallKind::Placement) => format!("the place block of {owner}"),
            Body::Calls(CallKind::Operation) => format!("the cut block of {owner}"),
            Body::Calls(CallKind::Step | CallKind::Selector) => {
                format!("a step block of {owner}")
            }
            _ => format!("the body of {owner}"),
        };
        self.node(BLOCK, |p| {
            let open = p.span(0);
            p.bump();
            let mut seen_place = false;
            loop {
                while p.at(NEWLINE) || p.at(COMMA) {
                    p.bump();
                }
                match p.current() {
                    R_BRACE => {
                        p.bump();
                        return Ok(());
                    }
                    EOF => {
                        p.missing_brace(&description, open);
                        return Ok(());
                    }
                    _ => {}
                }
                if let Some(keyword) = p.item_keyword_ahead()
                    && !body.allows_item(keyword)
                {
                    p.missing_brace(&description, open);
                    return Ok(());
                }
                let before = p.cursor;
                let result = p.entry(body, owner, &mut seen_place);
                if p.cursor == before {
                    p.skip_rest(true);
                    continue;
                }
                match p.current() {
                    NEWLINE | COMMA | R_BRACE | EOF => {}
                    _ if p.at_line_start() => {}
                    _ => {
                        if result.is_ok() {
                            let context = match body {
                                Body::Calls(call) => Some(call),
                                _ => None,
                            };
                            let help = p.missing_parentheses_help(context);
                            p.expected_with_help(&["end of line"], help.as_deref());
                        }
                        p.skip_rest(true);
                    }
                }
            }
        })
    }

    /// The item keyword that starts the current entry (after any doc
    /// comments), when it is not followed by `:`.
    fn item_keyword_ahead(&self) -> Option<SyntaxKind> {
        let mut n = 0;
        while self.nth(n) == DOC {
            n += 1;
        }
        let kind = self.nth(n);
        (kind.is_item_keyword() && self.nth(n + 1) != COLON).then_some(kind)
    }

    /// `true` if the current word starts a special entry rather than a field:
    /// it is `word` and not followed by `:`.
    fn at_entry_word(&self, word: &str) -> bool {
        self.at_word(word) && self.nth(1) != COLON
    }

    fn entry(&mut self, body: Body, owner: &str, seen_place: &mut bool) -> PResult {
        let result = match body {
            Body::Fields => self.field(),
            Body::Part | Body::Hardware => {
                if self.at(KW_PLACE) && self.nth(1) != COLON {
                    if *seen_place {
                        self.expected_with_help(
                            &["a field"],
                            Some("there is one place block; put every term in it"),
                        );
                    }
                    *seen_place = true;
                    self.place(owner)
                } else if body == Body::Hardware {
                    Err(self.expected_with_help(
                        &["'place'"],
                        Some("a hardware body holds only place { }; settings go inside the parentheses"),
                    ))
                } else {
                    self.field()
                }
            }
            Body::Kind => self.kind_entry(owner),
            Body::Recipe => {
                if self.at_entry_word("mark") || self.at_entry_word("cut") {
                    self.node(STEP_BLOCK, |p| {
                        p.bump();
                        p.block(Body::Calls(CallKind::Step), owner)
                    })
                } else {
                    self.field()
                }
            }
            Body::Profile => {
                if self.at(KW_AT) && self.nth(1) != COLON {
                    self.station()
                } else {
                    self.field()
                }
            }
            Body::Component => {
                if self.at(DOC) || self.current().is_design_item_keyword() {
                    return self.item(Scope::Component);
                }
                Err(self.expected(&["a declaration"]))
            }
            Body::Scene => {
                if self.at(KW_POSE) && self.nth(1) != COLON {
                    self.pose()
                } else {
                    self.field()
                }
            }
            Body::Roles => self.role(),
            Body::Member => self.member_entry(),
            Body::Mechanism => self.mechanism_entry(),
            Body::Calls(call) => self.call(call),
            Body::Patch => self.patch_op(),
        };
        if result.is_err() {
            self.skip_rest(true);
        }
        result
    }

    /// `name: expr`
    fn field(&mut self) -> PResult {
        if !self.nth_is_word(0) {
            if self.at(DOC) {
                return Err(self.expected_with_help(
                    &["a field"],
                    Some("doc comments go right before a declaration; use // for other comments"),
                ));
            }
            return Err(self.expected(&["a field"]));
        }
        if self.nth(1) != COLON {
            self.bump_error_token();
            return Err(self.expected(&["':'"]));
        }
        self.node(FIELD, |p| {
            let key = p.nth_text(0);
            let at = p.span(0).to(p.span(1));
            p.wrap(NAME);
            p.bump();
            let allowed = std::mem::replace(
                &mut p.interpolation_allowed,
                matches!(key, "note" | "check"),
            );
            let result = p.value(key, at);
            p.interpolation_allowed = allowed;
            result
        })
    }

    /// Consumes the current token into an error node, so a message can point
    /// at the token after it.
    fn bump_error_token(&mut self) {
        self.start(ERROR);
        self.bump();
        self.finish();
    }

    /// The value after `name:` or `name =`.
    fn value(&mut self, name: &str, at: Span) -> PResult {
        if self.at_value_end() {
            return Err(self.missing_value(name, at));
        }
        self.expr()
    }

    /// `place { terms }`
    fn place(&mut self, owner: &str) -> PResult {
        self.node(PLACE, |p| {
            p.bump();
            p.block(Body::Calls(CallKind::Placement), owner)
        })
    }

    fn kind_entry(&mut self, owner: &str) -> PResult {
        let mut n = 0;
        while self.nth(n) == DOC {
            n += 1;
        }
        let documented = self.nth(n);
        if matches!(documented, KW_PARAM | KW_REQUIRE) {
            return self.item(Scope::Kind);
        }
        if documented == IDENT && self.nth_text(n) == "makes" && self.nth(n + 1) != COLON {
            return self.makes();
        }
        if documented == IDENT && self.nth_text(n) == "member" && self.nth(n + 1) != COLON {
            return self.member();
        }
        if n > 0 {
            return Err(self.expected_with_help(
                &["'param'", "'require'", "'makes'", "'member'"],
                Some("doc comments go right before a declaration; use // for other comments"),
            ));
        }
        if self.at_mount() {
            return self.mount(owner);
        }
        if self.at_entry_word("mechanism") {
            return self.node(MECHANISM, |p| {
                p.bump();
                p.block(Body::Mechanism, owner)
            });
        }
        if self.at_entry_word("adjust") {
            return self.node(ADJUST, |p| {
                p.bump();
                p.root_name()?;
                p.expect_word("on")?;
                p.root_name()?;
                p.expect(COLON, "':'")?;
                let name = p.nth_text(0);
                let at = p.span(0);
                p.root_name()?;
                p.expect(KW_WITHIN, "'within'")?;
                p.value(name, at)
            });
        }
        if self.at_entry_word("roles") {
            return self.node(ROLES, |p| {
                p.bump();
                p.block(Body::Roles, owner)
            });
        }
        if self.at_entry_word("cut") {
            return self.node(CUT, |p| {
                p.bump();
                if p.nth(0) == IDENT && p.nth(1) == COLON {
                    // `cut dado: crossing(...) { }`: a named cut.
                    p.wrap(NAME);
                    p.bump();
                }
                p.call(CallKind::Selector)?;
                if !p.at(L_BRACE)
                    && let Some(help) = p.missing_parentheses_help(Some(CallKind::Selector))
                {
                    return Err(p.expected_with_help(&["'{'"], Some(&help)));
                }
                p.block(Body::Calls(CallKind::Operation), owner)
            });
        }
        self.field()
    }

    /// `{ DOC } makes part name : stock { ... }`
    fn makes(&mut self) -> PResult {
        self.node(MAKES, |p| {
            while p.at(DOC) {
                p.bump();
            }
            p.bump();
            p.expect(KW_PART, "'part'")?;
            p.declared_name()?;
            let owner = format!("part '{}'", p.previous_text());
            p.expect(COLON, "':'")?;
            p.reference()?;
            p.block(Body::Part, &owner)
        })
    }

    /// `{ DOC } member name [ : item ] [ { entries } ]`
    fn member(&mut self) -> PResult {
        self.node(MEMBER, |p| {
            while p.at(DOC) {
                p.bump();
            }
            p.bump();
            p.declared_name()?;
            if p.at(COLON) {
                p.bump();
                p.expr()?;
            }
            if p.at(L_BRACE) {
                let owner = format!("member '{}'", p.previous_text());
                p.block(Body::Member, &owner)?;
            }
            Ok(())
        })
    }

    /// `mount { fields }`: where a piece of hardware, or one of its
    /// members, sits.
    fn mount(&mut self, owner: &str) -> PResult {
        self.node(MOUNT, |p| {
            p.bump();
            p.block(Body::Fields, owner)
        })
    }

    /// `true` at `mount {`, the start of a mount block.
    fn at_mount(&self) -> bool {
        self.at_word("mount") && self.nth(1) == L_BRACE
    }

    /// `shape name: call`, `mount { }` or a field, in a member's body.
    fn member_entry(&mut self) -> PResult {
        if self.at_mount() {
            return self.mount("a member's mount");
        }
        if self.at_word("shape") && self.nth(1) == IDENT && self.nth(2) == COLON {
            return self.node(SHAPE, |p| {
                p.bump();
                let name = p.nth_text(0);
                let at = p.span(0).to(p.span(1));
                p.wrap(NAME);
                p.bump();
                p.value(name, at)
            });
        }
        self.field()
    }

    /// One entry of a `mechanism` block.
    fn mechanism_entry(&mut self) -> PResult {
        if self.at_entry_word("pivot") {
            return self.node(PIVOT, |p| {
                p.bump();
                p.member_pair()?;
                p.expect(KW_AT, "'at'")?;
                p.expr()
            });
        }
        if self.at_entry_word("slot") {
            return self.node(SLOT, |p| {
                p.bump();
                p.member_pair()?;
                p.expect_word("from")?;
                p.expr()?;
                p.expect_word("to")?;
                p.expr()
            });
        }
        if self.at_entry_word("rail") {
            return self.node(RAIL, |p| {
                p.bump();
                p.member_pair()?;
                p.expect_word("along")?;
                p.root_name()
            });
        }
        if self.at_entry_word("couple") {
            return self.node(COUPLE, |p| {
                p.bump();
                p.root_name()?;
                p.expect_word("to")?;
                let name = p.nth_text(0);
                let at = p.span(0).to(p.span(1));
                p.root_name()?;
                p.expect(COLON, "':'")?;
                p.value(name, at)
            });
        }
        if self.at_word("stop") && self.nth(1) == COLON {
            return self.node(STOP, |p| {
                let at = p.span(0).to(p.span(1));
                p.wrap(NAME);
                p.bump();
                p.value("stop", at)?;
                if !p.at(STRING_QUOTE) {
                    return Err(p.expected_with_help(
                        &["a string"],
                        Some("a stop is written: stop: value \"what stops it there\""),
                    ));
                }
                p.string();
                Ok(())
            });
        }
        self.field()
    }

    /// `a, b`: the two members of a mechanism entry.
    fn member_pair(&mut self) -> PResult {
        self.root_name()?;
        self.expect(COMMA, "','")?;
        self.root_name()
    }

    /// The contextual word `word`.
    fn expect_word(&mut self, word: &str) -> PResult {
        if self.at_word(word) {
            self.bump();
            Ok(())
        } else {
            let what = format!("'{word}'");
            Err(self.expected(&[what.as_str()]))
        }
    }

    /// `name: Type`
    fn role(&mut self) -> PResult {
        if !(self.nth_is_word(0) && self.nth(1) == COLON) {
            return Err(self.expected(&["a role"]));
        }
        self.node(ROLE, |p| {
            p.wrap(NAME);
            p.bump();
            p.type_name()
        })
    }

    /// `at station: value`
    fn station(&mut self) -> PResult {
        self.node(STATION, |p| {
            let start = p.span(0);
            p.bump();
            p.expr()?;
            let at = start.to(p.span(0));
            let written = p
                .text
                .get(
                    usize::try_from(start.start()).unwrap_or(0)
                        ..usize::try_from(p.span(0).start()).unwrap_or(0),
                )
                .unwrap_or("at")
                .trim_end()
                .to_owned();
            p.expect(COLON, "':'")?;
            p.value(&written, at)
        })
    }

    /// `pose path = value`
    fn pose(&mut self) -> PResult {
        self.node(POSE, |p| {
            p.bump();
            let start = p.span(0);
            p.path()?;
            let target = p.previous_path_text(start);
            let at = start.to(p.span(0));
            p.expect(EQ, "'='")?;
            p.value(&target, at)
        })
    }

    fn previous_path_text(&self, start: Span) -> String {
        let end = match self.cursor.checked_sub(1) {
            Some(previous) => self
                .significant
                .get(previous)
                .and_then(|&index| self.tokens.get(index))
                .map_or(start.end(), |token| token.span.end()),
            None => start.end(),
        };
        self.text
            .get(usize::try_from(start.start()).unwrap_or(0)..usize::try_from(end).unwrap_or(0))
            .unwrap_or("")
            .to_owned()
    }

    // -----------------------------------------------------------------------
    // Patch operations
    // -----------------------------------------------------------------------

    fn patch_op(&mut self) -> PResult {
        match self.current() {
            KW_ADD => self.node(ADD_OP, |p| {
                p.bump();
                if p.at(KW_IMPORT) {
                    p.import()?;
                } else if p.at(DOC) || p.at(KW_PRIVATE) || p.current().is_item_keyword() {
                    p.item(Scope::Patch)?;
                    if p.at_word("to") {
                        p.bump();
                        p.reference()?;
                    }
                } else {
                    return Err(p.expected(&["a declaration", "'import'"]));
                }
                if p.at_word("in") {
                    p.bump();
                    p.file_name()?;
                }
                Ok(())
            }),
            IDENT if self.at_word("replace") => self.node(REPLACE_OP, |p| {
                p.bump();
                if p.at(DOC) || p.at(KW_PRIVATE) || p.current().is_item_keyword() {
                    p.item(Scope::Patch)
                } else {
                    Err(p.expected(&["a declaration"]))
                }
            }),
            KW_SET => self.node(SET_OP, |p| {
                p.bump();
                let start = p.span(0);
                p.path()?;
                let target = p.previous_path_text(start);
                let at = start.to(p.span(0));
                p.expect(EQ, "'='")?;
                p.value(&target, at)
            }),
            IDENT if self.at_word("unset") => self.node(UNSET_OP, |p| {
                p.bump();
                p.path()
            }),
            KW_REMOVE => self.node(REMOVE_OP, |p| {
                p.bump();
                if p.eat(KW_IMPORT) {
                    p.file_name()?;
                    if p.at_word("in") {
                        p.bump();
                        p.file_name()?;
                    }
                    Ok(())
                } else if crate::ast::ItemTag::from_keyword(p.nth_text(0)).is_some() {
                    p.bump();
                    p.path()
                } else {
                    Err(p.expected(&["an item keyword", "'import'"]))
                }
            }),
            IDENT if self.at_word("rename") => self.node(RENAME_OP, |p| {
                p.bump();
                if crate::ast::ItemTag::from_keyword(p.nth_text(0)).is_none() {
                    return Err(p.expected(&["an item keyword"]));
                }
                p.bump();
                p.path()?;
                if !p.at_word("to") {
                    return Err(p.expected(&["'to'"]));
                }
                p.bump();
                p.declared_name()
            }),
            _ => Err(self.expected(&[
                "'add'",
                "'replace'",
                "'set'",
                "'unset'",
                "'remove'",
                "'rename'",
            ])),
        }
    }

    /// A string naming a file or a module source.
    fn file_name(&mut self) -> PResult {
        if self.at(STRING_QUOTE) {
            self.string();
            Ok(())
        } else {
            Err(self.expected(&["a file name in quotes"]))
        }
    }

    // -----------------------------------------------------------------------
    // Expressions
    // -----------------------------------------------------------------------

    fn expr(&mut self) -> PResult {
        self.expr_bp(BinaryPower::Lowest)
    }

    /// Precedence climbing: parses operators that bind tighter than `min`.
    fn expr_bp(&mut self, min: BinaryPower) -> PResult {
        let checkpoint = self.checkpoint();
        self.unary()?;
        let mut left_is_comparison = false;
        // Each operator wraps the expression so far in one more node, so a
        // long chain nests as deep as a long run of brackets.
        let mut chain: u32 = 0;
        loop {
            let Some(power) = BinaryPower::of(self.current()) else {
                return Ok(());
            };
            if power <= min {
                return Ok(());
            }
            chain += 1;
            if self.depth.saturating_add(chain) >= MAX_DEPTH {
                return Err(self.too_deep());
            }
            let comparison = power == BinaryPower::Compare;
            // Comparisons don't chain: `a < b < c` stops after `a < b`.
            if comparison && left_is_comparison {
                return Ok(());
            }
            self.node_at(checkpoint, BINARY_EXPR, |p| {
                p.bump();
                if p.at_value_end() {
                    return Err(p.expected(&["an expression"]));
                }
                p.expr_bp(power)
            })?;
            left_is_comparison = comparison;
        }
    }

    /// `expected a simpler expression`: the expression nests deeper than
    /// [`MAX_DEPTH`].
    fn too_deep(&mut self) -> Reported {
        self.expected_with_help(
            &["a simpler expression"],
            Some("split the expression with let"),
        )
    }

    fn unary(&mut self) -> PResult {
        if self.depth >= MAX_DEPTH {
            return Err(self.too_deep());
        }
        self.depth += 1;
        let result = if self.at(MINUS) || self.at(BANG) {
            self.node(UNARY_EXPR, |p| {
                p.bump();
                p.unary()
            })
        } else {
            self.postfix()
        };
        self.depth -= 1;
        result
    }

    fn postfix(&mut self) -> PResult {
        let checkpoint = self.checkpoint();
        self.primary()?;
        let mut chain: u32 = 0;
        loop {
            if matches!(self.current(), DOT | L_BRACK | L_PAREN) {
                chain += 1;
                if self.depth.saturating_add(chain) >= MAX_DEPTH {
                    return Err(self.too_deep());
                }
            }
            match self.current() {
                DOT => self.node_at(checkpoint, MEMBER_EXPR, |p| {
                    p.bump();
                    p.any_name()
                })?,
                L_BRACK => {
                    let _ = self.node_at(checkpoint, INDEX_EXPR, |p| {
                        p.index();
                        Ok(())
                    });
                }
                L_PAREN => {
                    let _ = self.node_at(checkpoint, CALL_EXPR, |p| {
                        p.arg_list();
                        Ok(())
                    });
                }
                _ => return Ok(()),
            }
        }
    }

    fn primary(&mut self) -> PResult {
        match self.current() {
            INT_NUMBER | DECIMAL_NUMBER
                if self.nth(1) == IDENT
                    && self.nth_text(1) == "in"
                    && matches!(self.nth(2), INT_NUMBER | DECIMAL_NUMBER) =>
            {
                // A slope: `1 in 8`.
                self.node(BINARY_EXPR, |p| {
                    p.wrap(LITERAL);
                    p.bump();
                    p.wrap(LITERAL);
                    Ok(())
                })
            }
            QUANTITY | INT_NUMBER | DECIMAL_NUMBER | KW_TRUE | KW_FALSE | KW_FIT => {
                self.wrap(LITERAL);
                Ok(())
            }
            STRING_QUOTE => {
                self.string();
                Ok(())
            }
            IDENT => {
                self.wrap(NAME_REF);
                Ok(())
            }
            L_BRACK => self.list(),
            L_PAREN => self.node(PAREN_EXPR, |p| {
                p.bump();
                let inside = p.expr();
                if inside.is_ok() && !p.at(R_PAREN) {
                    p.expected(&["')'"]);
                }
                p.skip_to_closer(R_PAREN);
                Ok(())
            }),
            _ => Err(self.expected(&["an expression"])),
        }
    }

    /// `[ a, b, c ]`
    fn list(&mut self) -> PResult {
        self.node(LIST_EXPR, |p| {
            p.bump();
            let inside = (|| {
                while !p.at(R_BRACK) {
                    p.expr()?;
                    if !p.eat(COMMA) {
                        if p.at(R_BRACK) {
                            break;
                        }
                        return Err(p.expected(&["','", "']'"]));
                    }
                }
                Ok(())
            })();
            let _ = inside;
            p.skip_to_closer(R_BRACK);
            Ok(())
        })
    }

    /// A string: quotes, text and `{name}` parts. The lexer has already
    /// checked its structure and reported an unterminated string.
    fn string(&mut self) {
        self.start(STRING);
        self.bump();
        loop {
            match self.current() {
                STRING_TEXT => self.bump(),
                INTERP_OPEN => self.interpolation(),
                _ => break,
            }
        }
        self.eat(STRING_QUOTE);
        self.finish();
    }

    fn interpolation(&mut self) {
        let span = self.span(0);
        self.start(INTERP);
        self.bump();
        let mut end = span;
        while self.nth_is_word(0) || self.at(DOT) {
            if self.at(DOT) {
                self.bump();
            } else {
                self.wrap(NAME);
            }
        }
        if self.at(INTERP_CLOSE) {
            end = self.span(0);
            self.bump();
        }
        self.finish();
        if !self.interpolation_allowed {
            let at = span.to(end);
            let written = self
                .text
                .get(
                    usize::try_from(at.start()).unwrap_or(0)
                        ..usize::try_from(at.end()).unwrap_or(0),
                )
                .unwrap_or("{")
                .to_owned();
            let diagnostic = Diagnostic::error(
                Code::Syntax,
                format!("expected plain text, found '{written}'"),
                Label::new(self.file, at),
            )
            .with_help("write \\{ for a literal brace; {name} works only in note:, check: and require messages");
            self.diagnostics.push(diagnostic);
        }
    }
}

/// The help line of a syntax error that has no more specific one: what
/// to write instead, worked out from what was expected. Every diagnostic
/// has a help line (grammar, "Diagnostics").
fn default_help(expectations: &[&str]) -> String {
    let first = expectations.first().copied().unwrap_or_default();
    match first {
        "a declaration" | "an item keyword" => {
            "a declaration starts on its own line with a keyword such as part, joint, region or let"
                .to_owned()
        }
        "a field" => "a field is written name: value, one to a line".to_owned(),
        "a name" => "write a name: letters, digits and _, starting with a letter or _".to_owned(),
        "an expression" => "write a value here, such as 19mm, a name or a call".to_owned(),
        "a role" => "a role is written name: Part or name: Parts".to_owned(),
        "a type" => "write a type, such as Length, Count or Part".to_owned(),
        _ if first.ends_with("in quotes") => format!("write {first}"),
        _ => format!("write {} here", expectations.join(" or ")),
    }
}

/// Binding power of the binary operators, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BinaryPower {
    Lowest,
    Or,
    And,
    Compare,
    Add,
    Multiply,
}

impl BinaryPower {
    fn of(kind: SyntaxKind) -> Option<BinaryPower> {
        Some(match kind {
            PIPE2 => BinaryPower::Or,
            AMP2 => BinaryPower::And,
            EQ2 | NEQ | LT | LE | GT | GE => BinaryPower::Compare,
            PLUS | MINUS => BinaryPower::Add,
            STAR | SLASH => BinaryPower::Multiply,
            _ => return None,
        })
    }
}
