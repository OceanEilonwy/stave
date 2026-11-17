//! Lowering expressions: names, members, indexes, calls and arithmetic.

use stave_lang::Code;
use stave_lang::ast::BaseKind;
use stave_lang::ast::{self, BinaryOp, ExprKind, Span, UnaryOp};
use stave_lang::placement::{Axis, Direction, Face, PartAxis};
use stave_lang::units::{number_to_rational, quantity_to_angle, quantity_to_length};
use stave_lang::{Diagnostic, Label};
use stave_schema::fields::ValueType;
use stave_schema::functions::{Builtin, MODEL_NODE};
use stave_schema::kinds::base;
use stave_schema::species::Species;
use stave_schema::words::{Grain, Measure, MotionWord, OutputUnit, StockForm, WordSet};

use super::{ArrayShape, Cx, Resolver};
use crate::defs::{
    DefId, Index, Member, RExpr, RExprKind, Setting, StockField, Symbol, Text, TextPart, Word,
};
use crate::types::{KindTy, Ty};

/// What a position expects.
#[derive(Debug, Clone, Default)]
pub(crate) struct Expect {
    /// The type, when the position has one.
    pub ty: Option<Ty>,
    /// The word lists a bare name may come from here.
    pub words: Vec<WordSet>,
    /// `true` if `fit` is allowed.
    pub fit: bool,
    /// The field or argument name, for messages.
    pub what: Option<String>,
}

impl Expect {
    pub fn any() -> Self {
        Expect::default()
    }

    pub fn ty(ty: Ty) -> Self {
        let words = match &ty {
            Ty::Word(set) => vec![*set],
            _ => Vec::new(),
        };
        Expect {
            ty: Some(ty),
            words,
            ..Expect::default()
        }
    }

    pub fn named(mut self, what: impl Into<String>) -> Self {
        self.what = Some(what.into());
        self
    }

    pub fn with_words(mut self, words: &[WordSet]) -> Self {
        self.words.extend_from_slice(words);
        self
    }

    /// What a schema value type expects.
    pub fn value_type(ty: ValueType) -> Self {
        match ty {
            ValueType::LengthOrFit => Expect {
                fit: true,
                ..Expect::ty(Ty::Length)
            },
            ValueType::Depth => Expect::ty(Ty::Length).with_words(&[WordSet::Through]),
            ValueType::Items => Expect::any(),
            other => match Ty::from_value_type(other) {
                Some(ty) => Expect::ty(ty),
                None => Expect::any(),
            },
        }
    }
}

/// The word for `text` in a word list.
pub(crate) fn make_word(set: WordSet, text: &str) -> Option<Word> {
    Some(match set {
        WordSet::Axis => Word::Axis(Axis::from_word(text)?),
        WordSet::Direction => Word::Direction(Direction::from_word(text)?),
        WordSet::Face => Word::Face(Face::from_word(text)?),
        WordSet::PartAxis => Word::PartAxis(PartAxis::from_word(text)?),
        WordSet::OutputUnit => Word::OutputUnit(OutputUnit::from_word(text)?),
        WordSet::Species => Word::Species(Species::from_word(text)?),
        WordSet::StockForm => Word::StockForm(StockForm::from_word(text)?),
        WordSet::Measure => Word::Measure(Measure::from_word(text)?),
        WordSet::Motion => Word::Motion(MotionWord::from_word(text)?),
        WordSet::Grain => Word::Grain(Grain::from_word(text)?),
        WordSet::Through if text == "through" => Word::Through,
        WordSet::Bounds if text == "bounds" => Word::Bounds,
        WordSet::Spacing if text == "evenly" => Word::Evenly,
        WordSet::Spacing if text == "ends" => Word::Ends,
        WordSet::Through | WordSet::Bounds | WordSet::Spacing => return None,
    })
}

fn word_ty(word: Word) -> Ty {
    match word {
        Word::Axis(_) => Ty::Word(WordSet::Axis),
        Word::Direction(_) => Ty::Word(WordSet::Direction),
        Word::Face(_) => Ty::Word(WordSet::Face),
        Word::PartAxis(_) => Ty::Word(WordSet::PartAxis),
        Word::OutputUnit(_) => Ty::Word(WordSet::OutputUnit),
        Word::Species(_) => Ty::Word(WordSet::Species),
        Word::StockForm(_) => Ty::Word(WordSet::StockForm),
        Word::Measure(_) => Ty::Word(WordSet::Measure),
        Word::Motion(_) => Ty::Word(WordSet::Motion),
        Word::Grain(_) => Ty::Word(WordSet::Grain),
        Word::Through => Ty::Word(WordSet::Through),
        Word::Bounds => Ty::Word(WordSet::Bounds),
        Word::Evenly | Word::Ends => Ty::Spacing,
    }
}

fn verb(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Sub => "subtract",
        BinaryOp::Mul => "multiply",
        BinaryOp::Div | BinaryOp::In => "divide",
        BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            "compare"
        }
        BinaryOp::And | BinaryOp::Or => "combine",
    }
}

/// The type of `lhs op rhs`, or `None` when the grammar's arithmetic table
/// has no such row.
pub(crate) fn binary_ty(op: BinaryOp, lhs: &Ty, rhs: &Ty) -> Option<Ty> {
    use Ty::{Angle, Bool, Count, Length, Ratio, Unknown};
    if matches!(lhs, Unknown) || matches!(rhs, Unknown) {
        return Some(match op {
            BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge => Bool,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => Unknown,
            BinaryOp::In => Ratio,
        });
    }
    let number = |t: &Ty| matches!(t, Count | Ratio);
    match op {
        BinaryOp::And | BinaryOp::Or => (lhs == &Bool && rhs == &Bool).then_some(Bool),
        BinaryOp::Eq | BinaryOp::Ne => {
            let same =
                lhs == rhs || (number(lhs) && number(rhs)) || (lhs.fits(rhs) && rhs.fits(lhs));
            same.then_some(Bool)
        }
        BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
            let ordered = matches!((lhs, rhs), (Length, Length) | (Angle, Angle))
                || (number(lhs) && number(rhs));
            ordered.then_some(Bool)
        }
        BinaryOp::Add | BinaryOp::Sub => match (lhs, rhs) {
            (Length, Length) => Some(Length),
            (Angle, Angle) => Some(Angle),
            (Count, Count) => Some(Count),
            (a, b) if number(a) && number(b) => Some(Ratio),
            _ => None,
        },
        BinaryOp::Mul => match (lhs, rhs) {
            (Length, b) | (b, Length) if number(b) => Some(Length),
            (Angle, b) | (b, Angle) if number(b) => Some(Angle),
            (Count, Count) => Some(Count),
            (a, b) if number(a) && number(b) => Some(Ratio),
            _ => None,
        },
        BinaryOp::Div => match (lhs, rhs) {
            (Length, b) if number(b) => Some(Length),
            (Angle, b) if number(b) => Some(Angle),
            (Length, Length) | (Angle, Angle) => Some(Ratio),
            (a, b) if number(a) && number(b) => Some(Ratio),
            _ => None,
        },
        BinaryOp::In => (number(lhs) && number(rhs)).then_some(Ratio),
    }
}

impl Resolver<'_> {
    /// Lowers an expression that is used as a value: arrays must be
    /// indexed, and the type must fit what the position expects.
    pub(crate) fn value(&mut self, cx: &Cx, expr: &ast::Expr, expect: &Expect) -> RExpr {
        let lowered = self.raw(cx, expr, expect);
        let lowered = self.reject_bare_array(cx, lowered);
        self.check_fits(cx, lowered, expect)
    }

    fn reject_bare_array(&mut self, cx: &Cx, expr: RExpr) -> RExpr {
        if let Ty::Array(inner) = &expr.ty {
            let name = self.expr_name(&expr);
            self.report(
                Diagnostic::error(
                    Code::TypeMismatch,
                    format!("'{name}' is an array of {}", plural(inner)),
                    Label::new(cx.file, expr.span),
                )
                .with_help(format!(
                    "write {name}[*] for all of them, or {name}[<label>] for one"
                )),
            );
            return RExpr::error(expr.span);
        }
        expr
    }

    fn expr_name(&self, expr: &RExpr) -> String {
        match &expr.kind {
            RExprKind::Symbol(Symbol::Def(def)) => self.def_name(*def),
            RExprKind::Member {
                member: Member::AssemblyItem(def),
                ..
            } => self.def_name(*def),
            _ => "this".into(),
        }
    }

    /// Reports when `expr` doesn't fit what is expected. Returns the
    /// expression (an error expression when it didn't fit).
    pub(crate) fn check_fits(&mut self, cx: &Cx, expr: RExpr, expect: &Expect) -> RExpr {
        let Some(wanted) = &expect.ty else {
            return expr;
        };
        if expr.ty == Ty::Fit {
            if expect.fit {
                return expr;
            }
            self.error_help(
                Code::TypeMismatch,
                format!(
                    "'{}' needs {wanted}, found 'fit'",
                    expect.what.as_deref().unwrap_or("this")
                ),
                cx.file,
                expr.span,
                "only the length or width of a part can be 'fit'",
            );
            return RExpr::error(expr.span);
        }
        if expr.ty.fits(wanted) || expect.words.iter().any(|set| expr.ty == Ty::Word(*set)) {
            return expr;
        }
        let what = expect.what.as_deref().unwrap_or("this value");
        let code = if expr.ty.is_quantity() && wanted.is_quantity() {
            Code::UnitMismatch
        } else {
            Code::TypeMismatch
        };
        self.error(
            code,
            format!("'{what}' needs {wanted}, found {}", expr.ty),
            cx.file,
            expr.span,
        );
        RExpr::error(expr.span)
    }

    /// Lowers an expression; the result may be an unindexed array.
    pub(crate) fn raw(&mut self, cx: &Cx, expr: &ast::Expr, expect: &Expect) -> RExpr {
        let span = expr.span;
        let make = |kind, ty| RExpr { kind, ty, span };
        match &expr.kind {
            ExprKind::Quantity(quantity) => match quantity.unit.dimension() {
                ast::Dimension::Length => match quantity_to_length(*quantity) {
                    Ok(length) => make(RExprKind::Length(length), Ty::Length),
                    Err(error) => self.too_large(cx, span, error),
                },
                ast::Dimension::Angle => match quantity_to_angle(*quantity) {
                    Ok(angle) => make(RExprKind::Angle(angle), Ty::Angle),
                    Err(error) => self.too_large(cx, span, error),
                },
            },
            ExprKind::Number(number) => match number_to_rational(*number) {
                Ok(value) => {
                    let ty = match number {
                        ast::Number::Integer(_) => Ty::Count,
                        ast::Number::Decimal(_) => Ty::Ratio,
                    };
                    make(RExprKind::Number(value), ty)
                }
                Err(error) => self.too_large(cx, span, error),
            },
            ExprKind::String(string) => {
                let text = self.text(cx, string);
                make(RExprKind::Text(text), Ty::Text)
            }
            ExprKind::Bool(value) => make(RExprKind::Bool(*value), Ty::Bool),
            ExprKind::Fit => make(RExprKind::Fit, Ty::Fit),
            ExprKind::Name(ident) => self.name(cx, ident, expect),
            ExprKind::Member { base, member } => self.member(cx, base, member, span),
            ExprKind::Index { base, index } => self.index(cx, base, index, span),
            ExprKind::Call { callee, args } => self.call(cx, callee, args, span, expect),
            ExprKind::Unary { op, operand } => {
                let operand_expect = match op {
                    UnaryOp::Not => Expect::ty(Ty::Bool).named("!"),
                    UnaryOp::Neg => Expect::any(),
                };
                let operand = self.value(cx, operand, &operand_expect);
                let ty = match (op, &operand.ty) {
                    (UnaryOp::Not, _) => Ty::Bool,
                    (
                        UnaryOp::Neg,
                        ty @ (Ty::Length | Ty::Angle | Ty::Ratio | Ty::Count | Ty::Unknown),
                    ) => ty.clone(),
                    (UnaryOp::Neg, other) => {
                        let message = format!("can't negate {other}");
                        self.error(Code::UnitMismatch, message, cx.file, span);
                        return RExpr::error(span);
                    }
                };
                make(
                    RExprKind::Unary {
                        op: *op,
                        operand: Box::new(operand),
                    },
                    ty,
                )
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let side_expect = match op {
                    BinaryOp::And | BinaryOp::Or => Expect::ty(Ty::Bool).named(op.symbol()),
                    _ => Expect::any(),
                };
                let lhs = self.value(cx, lhs, &side_expect);
                let rhs = self.value(cx, rhs, &side_expect);
                match binary_ty(*op, &lhs.ty, &rhs.ty) {
                    Some(ty) => make(
                        RExprKind::Binary {
                            op: *op,
                            lhs: Box::new(lhs),
                            rhs: Box::new(rhs),
                        },
                        ty,
                    ),
                    None => {
                        let help = match op {
                            BinaryOp::Add | BinaryOp::Sub => format!(
                                "only values of the same kind can be {}: write both as {}",
                                if *op == BinaryOp::Add { "added" } else { "subtracted" },
                                lhs.ty
                            ),
                            BinaryOp::Mul | BinaryOp::Div => "a length or angle can be multiplied or divided by a plain number, and divided by another length or angle".to_owned(),
                            _ => "compare values of the same kind: two lengths, two angles or two numbers".to_owned(),
                        };
                        self.error_help(
                            Code::UnitMismatch,
                            format!("can't {} {} and {}", verb(*op), lhs.ty, rhs.ty),
                            cx.file,
                            span,
                            help,
                        );
                        RExpr::error(span)
                    }
                }
            }
            ExprKind::List(elements) => {
                let element_expect = Expect {
                    ty: expect.ty.as_ref().and_then(|t| match t {
                        Ty::List(inner) => Some((**inner).clone()),
                        Ty::Parts => Some(Ty::Parts),
                        _ => None,
                    }),
                    words: expect.words.clone(),
                    fit: false,
                    what: expect.what.clone(),
                };
                let mut lowered = Vec::with_capacity(elements.len());
                let mut element_ty: Option<Ty> = None;
                for element in elements {
                    let value = self.value(cx, element, &element_expect);
                    // A list inside a list flattens; `x[*]` is a list.
                    let ty = match &value.ty {
                        Ty::List(inner) => (**inner).clone(),
                        Ty::Parts => Ty::Part,
                        other => other.clone(),
                    };
                    element_ty = Some(match element_ty {
                        None => ty,
                        Some(previous) if previous == ty => previous,
                        Some(previous) if ty.fits(&previous) => previous,
                        Some(previous) if previous.fits(&ty) => ty,
                        Some(_) => Ty::Unknown,
                    });
                    lowered.push(value);
                }
                let element_ty = element_ty.unwrap_or(Ty::Unknown);
                let ty = if element_ty == Ty::Part {
                    Ty::Parts
                } else {
                    Ty::List(Box::new(element_ty))
                };
                make(RExprKind::List(lowered), ty)
            }
            ExprKind::Paren(inner) => self.raw(cx, inner, expect),
            ExprKind::Range { .. } => {
                self.error_help(
                    Code::Syntax,
                    "a range lo..hi is written only as an argument of a member's shape",
                    cx.file,
                    span,
                    "write one value here; ranges size a hardware member's shape, as in box(across: -3mm..15mm, ...)",
                );
                RExpr::error(span)
            }
        }
    }

    fn too_large(&mut self, cx: &Cx, span: Span, error: stave_lang::units::UnitError) -> RExpr {
        self.error(Code::UnitMismatch, error.to_string(), cx.file, span);
        RExpr::error(span)
    }

    /// Text with `{name}` parts resolved in the surrounding scope.
    pub(crate) fn text(&mut self, cx: &Cx, string: &ast::StringLit) -> Text {
        let mut parts = Vec::new();
        for part in &string.parts {
            match part {
                ast::StringPart::Text { text, .. } => parts.push(TextPart::Literal(text.clone())),
                ast::StringPart::Interp {
                    root,
                    members,
                    span,
                } => {
                    let Ok(ident) = ast::Ident::new(root.as_str(), root.span()) else {
                        self.error(
                            Code::UnknownName,
                            format!("unknown name '{}'", root.as_str()),
                            cx.file,
                            root.span(),
                        );
                        continue;
                    };
                    let mut expr = ast::Expr {
                        kind: ExprKind::Name(ident),
                        span: root.span(),
                    };
                    for member in members {
                        expr = ast::Expr {
                            span: expr.span.to(member.span()),
                            kind: ExprKind::Member {
                                base: Box::new(expr),
                                member: member.clone(),
                            },
                        };
                    }
                    let mut value = self.value(cx, &expr, &Expect::any());
                    value.span = *span;
                    parts.push(TextPart::Value(value));
                }
            }
        }
        Text { parts }
    }

    /// A plain string (no interpolation) in a text position.
    pub(crate) fn plain_text(&mut self, cx: &Cx, expr: &ast::Expr, what: &str) -> Option<String> {
        if let ExprKind::String(string) = &expr.kind {
            return Some(self.text(cx, string).literal());
        }
        let found = self.value(cx, expr, &Expect::any());
        if found.ty != Ty::Unknown {
            self.error(
                Code::TypeMismatch,
                format!("'{what}' needs a text, found {}", found.ty),
                cx.file,
                expr.span,
            );
        }
        None
    }

    // -----------------------------------------------------------------------
    // Names
    // -----------------------------------------------------------------------

    fn name(&mut self, cx: &Cx, ident: &ast::Ident, expect: &Expect) -> RExpr {
        let text = ident.as_str();
        let span = ident.span();
        let item = self.lookup(cx, text);
        let word = expect.words.iter().find_map(|set| make_word(*set, text));
        if let Some(word) = word {
            if let Some(symbol) = item {
                // Only ambiguous when the position accepts the item too.
                let item_ty = self.symbol_ty(symbol);
                let accepts_item = match &expect.ty {
                    Some(Ty::Word(_)) | None => false,
                    Some(Ty::Spacing) => false,
                    Some(wanted) => item_ty.fits(wanted) && item_ty != Ty::Unknown,
                };
                if accepts_item {
                    self.report(
                        Diagnostic::error(
                            Code::AmbiguousName,
                            format!("'{text}' could be {} or the word '{text}' here", item_ty),
                            Label::new(cx.file, span),
                        )
                        .with_help(format!(
                            "rename the {} called '{text}'",
                            describe_symbol(symbol)
                        )),
                    );
                    return RExpr::error(span);
                }
            }
            return RExpr {
                kind: RExprKind::Word(word),
                ty: word_ty(word),
                span,
            };
        }
        match item {
            Some(Symbol::Def(def)) if cx.array == Some(def) && self.array_labels(def).is_some() => {
                // An unindexed array name inside its own declaration.
                let ty = match self.symbol_ty(Symbol::Def(def)) {
                    Ty::Array(inner) => *inner,
                    other => other,
                };
                RExpr {
                    kind: RExprKind::SelfElement(def),
                    ty,
                    span,
                }
            }
            Some(symbol) => {
                let ty = self.symbol_ty(symbol);
                RExpr {
                    kind: RExprKind::Symbol(symbol),
                    ty,
                    span,
                }
            }
            None if self.is_broken(cx, text) => RExpr::error(span),
            None if self.import(cx, text).is_some() => {
                self.error_help(
                    Code::UnknownName,
                    format!("unknown name '{text}'"),
                    cx.file,
                    span,
                    format!("'{text}' is a module; name one of its items: {text}.<name>"),
                );
                RExpr::error(span)
            }
            None => {
                // Where only a closed list of words fits, suggest only them.
                let only_words = match &expect.ty {
                    Some(Ty::Word(set)) => Some(*set),
                    _ => None,
                };
                let mut candidates = match only_words {
                    Some(set) => set.words().into_iter().map(str::to_owned).collect(),
                    None => self.names_in_scope(cx),
                };
                for set in &expect.words {
                    candidates.extend(set.words().into_iter().map(str::to_owned));
                }
                let fallback = match (&expect.ty, only_words) {
                    (_, Some(set)) => format!("write one of: {}", set.words().join(", ")),
                    (Some(wanted), None) if WordSet::ALL.iter().any(|s| s.contains(text)) => {
                        format!("this needs {wanted} here")
                    }
                    _ => format!("declare '{text}' in this module, or check its spelling"),
                };
                self.unknown(
                    Code::UnknownName,
                    format!("unknown name '{text}'"),
                    cx.file,
                    span,
                    text,
                    candidates,
                    fallback,
                );
                RExpr::error(span)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Members
    // -----------------------------------------------------------------------

    fn member(&mut self, cx: &Cx, base: &ast::Expr, member: &ast::Name, span: Span) -> RExpr {
        // `module.item`: a qualified name.
        if let ExprKind::Name(root) = &base.kind
            && self.lookup(cx, root.as_str()).is_none()
            && let Some(module) = self.import(cx, root.as_str())
        {
            {
                {
                    return match self.public_item(module, member.as_str()) {
                        Ok(def) => {
                            let ty = self.symbol_ty(Symbol::Def(def));
                            RExpr {
                                kind: RExprKind::Symbol(Symbol::Def(def)),
                                ty,
                                span,
                            }
                        }
                        Err(false)
                            if self
                                .broken
                                .get(&module)
                                .is_some_and(|names| names.contains(member.as_str())) =>
                        {
                            RExpr::error(span)
                        }
                        Err(false) => {
                            let candidates = self.public_names(module);
                            let fallback = format!(
                                "module '{}' has no public item called '{}'",
                                self.module_name(module),
                                member.as_str()
                            );
                            self.unknown(
                                Code::UnknownName,
                                format!("unknown name '{}.{}'", root.as_str(), member.as_str()),
                                cx.file,
                                span,
                                member.as_str(),
                                candidates,
                                fallback,
                            );
                            RExpr::error(span)
                        }
                        Err(private) => {
                            let mut diagnostic = Diagnostic::error(
                                Code::UnknownName,
                                format!("unknown name '{}.{}'", root.as_str(), member.as_str()),
                                Label::new(cx.file, span),
                            );
                            diagnostic = if private {
                                diagnostic.with_help(format!(
                                    "'{}' is private to module '{}'",
                                    member.as_str(),
                                    self.module_name(module)
                                ))
                            } else {
                                diagnostic.with_help(format!(
                                    "module '{}' has no public item called '{}'",
                                    self.module_name(module),
                                    member.as_str()
                                ))
                            };
                            self.report(diagnostic);
                            RExpr::error(span)
                        }
                    };
                }
            }
        }
        let base = self.raw(cx, base, &Expect::any());
        let base = if matches!(base.ty, Ty::Array(_)) {
            self.reject_bare_array(cx, base)
        } else {
            base
        };
        if base.kind == RExprKind::Error {
            return RExpr::error(span);
        }
        let name = member.as_str();
        let resolved: Option<(Member, Ty)> = match &base.ty {
            Ty::Part => match name {
                "length" => Some((Member::Size(PartAxis::Length), Ty::Length)),
                "width" => Some((Member::Size(PartAxis::Width), Ty::Length)),
                "thickness" => Some((Member::Size(PartAxis::Thickness), Ty::Length)),
                "diameter" => Some((Member::Diameter, Ty::Length)),
                "stock" => Some((Member::PartStock, Ty::Stock)),
                _ => Face::from_word(name).map(|face| (Member::Face(face), Ty::Anchor)),
            },
            Ty::Region => match Axis::from_word(name) {
                Some(axis) => Some((Member::RegionSize(axis), Ty::Length)),
                None => Face::from_word(name).map(|face| (Member::Face(face), Ty::Anchor)),
            },
            Ty::Stock => StockField::from_name(name).map(|field| {
                let ty = match field {
                    StockField::Species => Ty::Word(WordSet::Species),
                    StockField::Form => Ty::Word(WordSet::StockForm),
                    _ => Ty::Length,
                };
                (Member::StockField(field), ty)
            }),
            Ty::Profile => (name == "length").then_some((Member::ProfileLength, Ty::Length)),
            Ty::Assembly(component) => {
                let entry = self
                    .component_names
                    .get(component.index())
                    .and_then(|names| names.get(name))
                    .map(|entry| entry.def);
                match entry {
                    Some(def) => {
                        let ty = self.symbol_ty(Symbol::Def(def));
                        Some((Member::AssemblyItem(def), ty))
                    }
                    None => None,
                }
            }
            Ty::Kind(kind) => self.kind_member(*kind, name),
            Ty::Unknown => Some((Member::KindEntry(name.to_owned()), Ty::Unknown)),
            _ => None,
        };
        match resolved {
            Some((member, ty)) => RExpr {
                kind: RExprKind::Member {
                    base: Box::new(base),
                    member,
                },
                ty,
                span,
            },
            None if matches!(&base.ty, Ty::Assembly(component) if self.broken_in_component(*component, name)) =>
            {
                // An item of the component left out for a syntax error.
                RExpr::error(span)
            }
            None => {
                let what = match &base.ty {
                    Ty::Assembly(component) => {
                        format!(
                            "the component '{}'",
                            self.def_name(DefId::Component(*component))
                        )
                    }
                    other => other.to_string(),
                };
                let candidates = self.member_names(&base.ty);
                self.unknown(
                    Code::UnknownField,
                    format!("{what} has no field '{name}'"),
                    cx.file,
                    member.span(),
                    name,
                    candidates,
                    member_help(&base.ty),
                );
                RExpr::error(span)
            }
        }
    }

    /// `true` if the component's item `name` was left out for a syntax
    /// error.
    pub(crate) fn broken_in_component(
        &self,
        component: crate::ids::ComponentId,
        name: &str,
    ) -> bool {
        let component_name = self.def_name(DefId::Component(component));
        self.origins
            .get(&DefId::Component(component))
            .and_then(|origin| self.broken.get(&origin.module))
            .is_some_and(|names| names.contains(&format!("{component_name}.{name}")))
    }

    /// The readable members of a type, for suggestions.
    fn member_names(&self, ty: &Ty) -> Vec<String> {
        let faces = Face::ALL.iter().map(|f| f.word().to_owned());
        match ty {
            Ty::Part => ["length", "width", "thickness", "diameter", "stock"]
                .iter()
                .map(|s| (*s).to_owned())
                .chain(faces)
                .collect(),
            Ty::Region => Axis::ALL
                .iter()
                .map(|a| a.word().to_owned())
                .chain(faces)
                .collect(),
            Ty::Stock => [
                "species",
                "form",
                "thickness",
                "diameter",
                "width",
                "max_width",
                "max_length",
            ]
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
            Ty::Profile => vec!["length".to_owned()],
            Ty::Assembly(component) => self
                .component_names
                .get(component.index())
                .map(|names| names.keys().cloned().collect())
                .unwrap_or_default(),
            Ty::Kind(KindTy {
                kind: Some(kind), ..
            }) => match self.origins.get(&DefId::Kind(*kind)).map(|o| o.syntax) {
                Some(super::Syntax::Kind(syntax)) => super::kinds::kind_params(syntax)
                    .iter()
                    .map(|p| p.name.as_str().to_owned())
                    .chain(syntax.body.entries.iter().filter_map(|e| match e {
                        ast::KindEntry::Field(f) => Some(f.key.as_str().to_owned()),
                        _ => None,
                    }))
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn kind_member(&mut self, kind: KindTy, name: &str) -> Option<(Member, Ty)> {
        let member = Member::KindEntry(name.to_owned());
        if let Some(id) = kind.kind {
            let origin = self.origins.get(&DefId::Kind(id)).copied()?;
            let super::Syntax::Kind(syntax) = origin.syntax else {
                return None;
            };
            if let Some(index) = super::kinds::kind_params(syntax)
                .iter()
                .position(|p| p.name.as_str() == name)
            {
                let ty = self.kind_param_ty(id, index);
                return Some((member, ty));
            }
            if syntax.body.field(name).is_some() {
                let ty = base(syntax.base.value)
                    .entry(name)
                    .and_then(|spec| Ty::from_value_type(spec.ty))
                    .unwrap_or(Ty::Unknown);
                return Some((member, ty));
            }
            if let Some(spec) = base(syntax.base.value).entry(name) {
                // An optional entry the kind doesn't set.
                return Some((member, Ty::from_value_type(spec.ty).unwrap_or(Ty::Unknown)));
            }
            return None;
        }
        match kind.base {
            Some(base_kind) => match base(base_kind).entry(name) {
                Some(spec) => Some((member, Ty::from_value_type(spec.ty).unwrap_or(Ty::Unknown))),
                // A param of whichever kind is passed in.
                None => Some((member, Ty::Unknown)),
            },
            None => Some((member, Ty::Unknown)),
        }
    }

    // -----------------------------------------------------------------------
    // Indexes
    // -----------------------------------------------------------------------

    fn index(&mut self, cx: &Cx, base: &ast::Expr, index: &ast::Index, span: Span) -> RExpr {
        let base = self.raw(cx, base, &Expect::any());
        if base.kind == RExprKind::Error {
            return RExpr::error(span);
        }
        let Ty::Array(element) = base.ty.clone() else {
            self.error_help(
                Code::TypeMismatch,
                format!("'{}' is not an array", self.expr_name(&base)),
                cx.file,
                base.span,
                "only arrays declared with [labels] or [count] can be indexed",
            );
            return RExpr::error(span);
        };
        let array_def = match &base.kind {
            RExprKind::Symbol(Symbol::Def(def)) => Some(*def),
            RExprKind::Member {
                member: Member::AssemblyItem(def),
                ..
            } => Some(*def),
            _ => None,
        };
        let shape = array_def.and_then(|def| self.array_labels(def));
        let element = *element;
        let (resolved, ty) = match index {
            ast::Index::All(_) => {
                let ty = if element == Ty::Part {
                    Ty::Parts
                } else {
                    Ty::List(Box::new(element))
                };
                (Index::All, ty)
            }
            ast::Index::Same(same_span) => {
                if cx.array.is_none() {
                    self.error_help(
                        Code::TypeMismatch,
                        "'[same]' is only allowed inside the declaration of an array",
                        cx.file,
                        *same_span,
                        "name the element instead: name[label]",
                    );
                    return RExpr::error(span);
                }
                (Index::Same, element)
            }
            ast::Index::At(at) => match shape {
                Some(ArrayShape::Labels(labels)) => {
                    let label = match &at.kind {
                        ExprKind::Name(ident) => Some(ident.as_str().to_owned()),
                        ExprKind::Number(ast::Number::Integer(n)) => Some(n.to_string()),
                        _ => None,
                    };
                    match label
                        .as_deref()
                        .and_then(|l| labels.iter().position(|x| x == l))
                    {
                        Some(position) => (Index::Label(position), element),
                        None => {
                            let written = label.unwrap_or_else(|| "this".into());
                            self.unknown(
                                Code::UnknownName,
                                format!("unknown name '{written}'"),
                                cx.file,
                                at.span,
                                &written,
                                labels.iter(),
                                format!(
                                    "'{}' is indexed by its labels: {}",
                                    self.expr_name(&base),
                                    labels.join(", ")
                                ),
                            );
                            return RExpr::error(span);
                        }
                    }
                }
                Some(ArrayShape::Count) | None => {
                    let number = self.value(cx, at, &Expect::ty(Ty::Count).named("index"));
                    (Index::Number(Box::new(number)), element)
                }
            },
        };
        RExpr {
            kind: RExprKind::Index {
                base: Box::new(base),
                index: resolved,
            },
            ty,
            span,
        }
    }

    // -----------------------------------------------------------------------
    // Calls
    // -----------------------------------------------------------------------

    fn call(
        &mut self,
        cx: &Cx,
        callee: &ast::Expr,
        args: &[ast::Arg],
        span: Span,
        expect: &Expect,
    ) -> RExpr {
        if let ExprKind::Name(ident) = &callee.kind {
            let item = self.lookup(cx, ident.as_str());
            let item_ty = item.map(|symbol| self.symbol_ty(symbol));
            let is_kind = matches!(item_ty, Some(Ty::Kind(_)));
            if !is_kind {
                if item.is_none()
                    && stave_schema::functions::is_member_only_function(ident.as_str())
                {
                    self.error_help(
                        Code::Syntax,
                        format!(
                            "{}(...) is written only inside a hardware kind's member or mechanism",
                            ident.as_str()
                        ),
                        cx.file,
                        span,
                        "points and member shapes are measurements within the hardware, in the kind's own frame; a design places things with relationships",
                    );
                    return RExpr::error(span);
                }
                if let Some(builtin) = Builtin::from_word(ident.as_str()) {
                    return self.builtin(cx, builtin, args, span, expect);
                }
                if ident.as_str() == MODEL_NODE {
                    // node("mount") is only meaningful as model(align:).
                    return self.node(cx, args, span);
                }
            }
        }
        let kind = self.value(cx, callee, &Expect::any());
        match kind.ty.clone() {
            Ty::Kind(kind_ty) => {
                let settings = self.kind_settings(cx, kind_ty, args, &[]);
                RExpr {
                    kind: RExprKind::KindValue {
                        kind: Box::new(kind),
                        settings,
                    },
                    ty: Ty::Kind(kind_ty),
                    span,
                }
            }
            Ty::Unknown => RExpr::error(span),
            other => {
                self.error_help(
                    Code::TypeMismatch,
                    format!("{other} can't be called"),
                    cx.file,
                    callee.span,
                    "only kinds and built-in functions take arguments",
                );
                RExpr::error(span)
            }
        }
    }

    fn node(&mut self, cx: &Cx, args: &[ast::Arg], span: Span) -> RExpr {
        let mut settings = Vec::new();
        for arg in args {
            let value = self.value(cx, &arg.value, &Expect::ty(Ty::Text).named("node"));
            settings.push(Setting {
                name: "name".into(),
                name_span: arg.span,
                value,
            });
        }
        RExpr {
            kind: RExprKind::Builtin {
                function: Builtin::Model,
                args: settings,
            },
            ty: Ty::Text,
            span,
        }
    }

    /// The settings of a kind value: every argument named, each a param of
    /// the kind (or `extra`).
    pub(crate) fn kind_settings(
        &mut self,
        cx: &Cx,
        kind: KindTy,
        args: &[ast::Arg],
        extra: &[(&str, Ty)],
    ) -> Vec<Setting> {
        let mut settings: Vec<Setting> = Vec::new();
        for arg in args {
            let Some(name) = &arg.name else {
                self.error_help(
                    Code::TypeMismatch,
                    "a kind value takes its settings by name",
                    cx.file,
                    arg.span,
                    "roles are given where the kind is used in a joint or hardware declaration",
                );
                continue;
            };
            if settings.iter().any(|s| s.name == name.as_str()) {
                self.error(
                    Code::DuplicateName,
                    format!("'{}' is declared twice", name.as_str()),
                    cx.file,
                    name.span(),
                );
                continue;
            }
            let expected = match extra.iter().find(|(n, _)| *n == name.as_str()) {
                Some((_, ty)) => Some(ty.clone()),
                None => self.setting_ty(cx, kind, name),
            };
            let Some(expected) = expected else { continue };
            let value = self.value(cx, &arg.value, &Expect::ty(expected).named(name.as_str()));
            settings.push(Setting {
                name: name.as_str().to_owned(),
                name_span: name.span(),
                value,
            });
        }
        settings
    }

    /// The type of a setting of a kind; reports unknown settings and roles.
    pub(crate) fn setting_ty(&mut self, cx: &Cx, kind: KindTy, name: &ast::Name) -> Option<Ty> {
        let Some(id) = kind.kind else {
            return Some(Ty::Unknown);
        };
        let origin = self.origins.get(&DefId::Kind(id)).copied()?;
        let super::Syntax::Kind(syntax) = origin.syntax else {
            return None;
        };
        let params = super::kinds::kind_params(syntax);
        if let Some(index) = params.iter().position(|p| p.name.as_str() == name.as_str()) {
            return Some(self.kind_param_ty(id, index));
        }
        if syntax.base.value == BaseKind::Joint
            && let Some(spec) = stave_schema::fields::find(
                stave_schema::fields::JOINT_IMPLICIT_SETTINGS,
                name.as_str(),
            )
        {
            return Ty::from_value_type(spec.ty);
        }
        if syntax.base.value.is_mounted_hardware()
            && name.as_str() == stave_schema::kinds::FACE_SETTING
        {
            return Some(Ty::Word(WordSet::Face));
        }
        let is_role = super::kinds::role_specs(syntax)
            .iter()
            .any(|(role, _, _)| role == name.as_str());
        let message = if is_role {
            format!(
                "'{}' is a role of '{}'; a kind value has no roles",
                name.as_str(),
                self.kind_display(id, cx.file)
            )
        } else {
            format!(
                "the kind '{}' has no field '{}'",
                self.kind_display(id, cx.file),
                name.as_str()
            )
        };
        let help = format!(
            "its settings are: {}",
            params
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut candidates: Vec<String> =
            params.iter().map(|p| p.name.as_str().to_owned()).collect();
        candidates.extend(
            super::kinds::role_specs(syntax)
                .into_iter()
                .map(|(role, _, _)| role),
        );
        if syntax.base.value == BaseKind::Joint {
            candidates.extend(["glue".to_owned(), "methods".to_owned()]);
        }
        self.unknown(
            Code::UnknownField,
            message,
            cx.file,
            name.span(),
            name.as_str(),
            candidates,
            help,
        );
        None
    }

    fn builtin(
        &mut self,
        cx: &Cx,
        builtin: Builtin,
        args: &[ast::Arg],
        span: Span,
        _expect: &Expect,
    ) -> RExpr {
        let parameters = builtin.parameters();
        let mut settings: Vec<Setting> = Vec::new();
        let mut misspelt: Vec<String> = Vec::new();
        let mut positional = parameters.iter().filter(|p| p.positional);
        for arg in args {
            let parameter = match &arg.name {
                None => match positional.next() {
                    Some(parameter) => parameter,
                    None => {
                        self.error_help(
                            Code::TypeMismatch,
                            format!(
                                "'{}' takes no more arguments without a name",
                                builtin.word()
                            ),
                            cx.file,
                            arg.span,
                            format!(
                                "write it as name: value; {} takes {}",
                                builtin.word(),
                                signature(builtin)
                            ),
                        );
                        continue;
                    }
                },
                Some(name) => match parameters
                    .iter()
                    .find(|p| !p.positional && p.name == name.as_str())
                {
                    Some(parameter) => parameter,
                    None => {
                        misspelt.push(name.as_str().to_owned());
                        self.unknown(
                            Code::UnknownField,
                            format!("'{}' has no field '{}'", builtin.word(), name.as_str()),
                            cx.file,
                            name.span(),
                            name.as_str(),
                            parameters.iter().filter(|p| !p.positional).map(|p| p.name),
                            format!("{} takes {}", builtin.word(), signature(builtin)),
                        );
                        continue;
                    }
                },
            };
            if settings.iter().any(|s| s.name == parameter.name) {
                self.error(
                    Code::DuplicateName,
                    format!("'{}' is declared twice", parameter.name),
                    cx.file,
                    arg.span,
                );
                continue;
            }
            let mut expect = Expect::value_type(parameter.ty).named(parameter.name);
            if builtin == Builtin::Model && parameter.name == "align" {
                expect = Expect::any().with_words(&[WordSet::Bounds]).named("align");
            }
            // `extent(into, thickness, of: piece)`: with `of:`, the axis is
            // one of that part's own axes.
            if builtin == Builtin::Extent
                && parameter.name == "axis"
                && args
                    .iter()
                    .any(|a| a.name.as_ref().is_some_and(|n| n.as_str() == "of"))
            {
                expect = Expect::ty(Ty::Word(WordSet::PartAxis)).named("axis");
            }
            let value = match parameter.ty {
                ValueType::Items => {
                    let value = self.value(cx, &arg.value, &Expect::any().named(parameter.name));
                    self.check_item_argument(cx, builtin, &value);
                    value
                }
                _ => self.value(cx, &arg.value, &expect),
            };
            settings.push(Setting {
                name: parameter.name.to_owned(),
                name_span: arg.span,
                value,
            });
        }
        for parameter in parameters.iter().filter(|p| p.required) {
            let misspelling = misspelt
                .iter()
                .any(|m| !crate::suggest::closest(m, [parameter.name]).is_empty());
            if !settings.iter().any(|s| s.name == parameter.name) && !misspelling {
                self.error_help(
                    Code::MissingField,
                    format!("'{}' is missing '{}'", builtin.word(), parameter.name),
                    cx.file,
                    span,
                    format!("{} takes {}", builtin.word(), signature(builtin)),
                );
            }
        }
        let ty = match builtin.returns() {
            _ if builtin == Builtin::Split => Ty::Regions,
            name => Ty::from_type_name(name),
        };
        RExpr {
            kind: RExprKind::Builtin {
                function: builtin,
                args: settings,
            },
            ty,
            span,
        }
    }

    fn check_item_argument(&mut self, cx: &Cx, builtin: Builtin, value: &RExpr) {
        let fits = match builtin {
            Builtin::Mid => matches!(value.ty, Ty::Part | Ty::Region | Ty::Unknown),
            Builtin::Between => matches!(value.ty, Ty::Part | Ty::Anchor | Ty::Unknown),
            Builtin::Extent => value.ty.fits(&Ty::Parts),
            _ => true,
        };
        if !fits {
            let wanted = match builtin {
                Builtin::Mid => "a part or a region",
                Builtin::Between => "a part or a face",
                _ => "parts",
            };
            self.error(
                Code::TypeMismatch,
                format!("'{}' needs {wanted}, found {}", builtin.word(), value.ty),
                cx.file,
                value.span,
            );
        }
    }

    /// Lowers a `ref` as an expression (stock, component and kind refs).
    pub(crate) fn reference(&mut self, cx: &Cx, reference: &ast::Ref, expect: &Expect) -> RExpr {
        let mut expr = ast::Expr {
            kind: ExprKind::Name(reference.root.clone()),
            span: reference.root.span(),
        };
        for member in &reference.rest {
            expr = ast::Expr {
                span: expr.span.to(member.span()),
                kind: ExprKind::Member {
                    base: Box::new(expr),
                    member: member.clone(),
                },
            };
        }
        self.value(cx, &expr, expect)
    }
}

fn plural(ty: &Ty) -> String {
    let text = ty.to_string();
    let bare = text
        .strip_prefix("a ")
        .or_else(|| text.strip_prefix("an "))
        .unwrap_or(&text)
        .to_owned();
    if bare.ends_with('s') {
        bare
    } else {
        format!("{bare}s")
    }
}

fn describe_symbol(symbol: Symbol) -> &'static str {
    match symbol {
        Symbol::Def(def) => super::def_word(def),
        Symbol::Role(..) => "role",
        Symbol::KindParam(..) => "param",
        Symbol::MadePart(..) => "made part",
    }
}

fn member_help(ty: &Ty) -> String {
    match ty {
        Ty::Part => "a part has length, width, thickness, diameter, stock and the faces top, bottom, left, right, front and back".into(),
        Ty::Region => "a region has across, up, deep and the faces top, bottom, left, right, front and back".into(),
        Ty::Stock => "a stock has species, form, thickness, diameter, width, max_width and max_length".into(),
        Ty::Profile => "a profile has length".into(),
        _ => format!("{ty} has no readable fields here"),
    }
}

fn signature(builtin: Builtin) -> String {
    let parts: Vec<String> = builtin
        .parameters()
        .iter()
        .map(|p| {
            if p.positional {
                p.name.to_owned()
            } else {
                format!("{}:", p.name)
            }
        })
        .collect();
    format!("{}({})", builtin.word(), parts.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_follows_the_grammar_table() {
        use BinaryOp::*;
        assert_eq!(binary_ty(Add, &Ty::Length, &Ty::Length), Some(Ty::Length));
        assert_eq!(binary_ty(Add, &Ty::Length, &Ty::Angle), None);
        assert_eq!(binary_ty(Mul, &Ty::Count, &Ty::Length), Some(Ty::Length));
        assert_eq!(binary_ty(Div, &Ty::Length, &Ty::Count), Some(Ty::Length));
        assert_eq!(binary_ty(Div, &Ty::Length, &Ty::Length), Some(Ty::Ratio));
        assert_eq!(binary_ty(Div, &Ty::Count, &Ty::Length), None);
        assert_eq!(binary_ty(Sub, &Ty::Count, &Ty::Length), None);
        assert_eq!(binary_ty(Le, &Ty::Length, &Ty::Length), Some(Ty::Bool));
        assert_eq!(binary_ty(Le, &Ty::Length, &Ty::Count), None);
        assert_eq!(binary_ty(And, &Ty::Bool, &Ty::Bool), Some(Ty::Bool));
        assert_eq!(binary_ty(Div, &Ty::Count, &Ty::Count), Some(Ty::Ratio));
        assert_eq!(binary_ty(Mul, &Ty::Count, &Ty::Count), Some(Ty::Count));
    }

    #[test]
    fn words_resolve_for_the_expected_list() {
        assert_eq!(make_word(WordSet::Axis, "up"), Some(Word::Axis(Axis::Up)));
        assert_eq!(
            make_word(WordSet::Direction, "up"),
            Some(Word::Direction(Direction::Up))
        );
        assert_eq!(make_word(WordSet::Face, "up"), None);
        assert_eq!(make_word(WordSet::Spacing, "ends"), Some(Word::Ends));
        assert_eq!(make_word(WordSet::Through, "through"), Some(Word::Through));
    }
}
