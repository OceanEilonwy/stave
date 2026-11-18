//! Expressions back to `.stave` text, for the values a generated patch
//! writes: a setting copied from another assembly or chosen in the card.
//!
//! The text parses back to the same expression. Brackets the source wrote
//! are kept (the tree keeps them as [`ExprKind::Paren`]); brackets are
//! added only where a hand-built tree needs them to keep its meaning.

use stave_lang::ast::{
    Arg, Decimal, Expr, ExprKind, Index, Magnitude, Number, Quantity, StringLit, StringPart,
};

/// An expression as `.stave` text: `overlay(left: 18mm, meeting: -reveal / 2)`.
pub fn expr_text(expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, expr);
    out
}

/// A string literal as `.stave` text, quotes included, with `"`, `\`, `{`
/// and line breaks escaped and each `{name}` part written back.
pub fn string_text(literal: &StringLit) -> String {
    let mut out = String::from("\"");
    for part in &literal.parts {
        match part {
            StringPart::Text { text, .. } => escape_into(&mut out, text),
            StringPart::Interp { root, members, .. } => {
                out.push('{');
                out.push_str(root.as_str());
                for member in members {
                    out.push('.');
                    out.push_str(member.as_str());
                }
                out.push('}');
            }
        }
    }
    out.push('"');
    out
}

/// Plain text as a `.stave` string literal, quotes included.
pub fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    escape_into(&mut out, text);
    out.push('"');
    out
}

fn escape_into(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
}

/// How tightly an expression binds, for deciding where brackets go:
/// higher binds tighter.
fn binding(expr: &Expr) -> u8 {
    match &expr.kind {
        ExprKind::Binary { op, .. } => op.precedence(),
        ExprKind::Range { .. } => 0,
        ExprKind::Unary { .. } => u8::MAX - 1,
        ExprKind::Quantity(_)
        | ExprKind::Number(_)
        | ExprKind::String(_)
        | ExprKind::Bool(_)
        | ExprKind::Fit
        | ExprKind::Name(_)
        | ExprKind::Member { .. }
        | ExprKind::Index { .. }
        | ExprKind::Call { .. }
        | ExprKind::List(_)
        | ExprKind::Paren(_) => u8::MAX,
    }
}

fn write_operand(out: &mut String, expr: &Expr, needs_at_least: u8) {
    if binding(expr) < needs_at_least {
        out.push('(');
        write_expr(out, expr);
        out.push(')');
    } else {
        write_expr(out, expr);
    }
}

fn write_expr(out: &mut String, expr: &Expr) {
    match &expr.kind {
        ExprKind::Quantity(quantity) => write_quantity(out, quantity),
        ExprKind::Number(number) => match number {
            Number::Integer(n) => out.push_str(&n.to_string()),
            Number::Decimal(decimal) => write_decimal(out, *decimal),
        },
        ExprKind::String(literal) => out.push_str(&string_text(literal)),
        ExprKind::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
        ExprKind::Fit => out.push_str("fit"),
        ExprKind::Name(name) => out.push_str(name.as_str()),
        ExprKind::Member { base, member } => {
            write_operand(out, base, u8::MAX);
            out.push('.');
            out.push_str(member.as_str());
        }
        ExprKind::Index { base, index } => {
            write_operand(out, base, u8::MAX);
            out.push('[');
            match index {
                Index::All(_) => out.push('*'),
                Index::Same(_) => out.push_str("same"),
                Index::At(at) => write_expr(out, at),
            }
            out.push(']');
        }
        ExprKind::Call { callee, args } => {
            write_operand(out, callee, u8::MAX);
            out.push('(');
            write_args(out, args);
            out.push(')');
        }
        ExprKind::Unary { op, operand } => {
            out.push_str(op.symbol());
            write_operand(out, operand, u8::MAX - 1);
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let precedence = op.precedence();
            // Operators of one level group from the left, so a right
            // operand of the same level needs brackets; comparisons don't
            // chain at all.
            let left_needs = if op.is_comparison() {
                precedence + 1
            } else {
                precedence
            };
            write_operand(out, lhs, left_needs);
            out.push(' ');
            out.push_str(op.symbol());
            out.push(' ');
            write_operand(out, rhs, precedence + 1);
        }
        ExprKind::List(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_expr(out, item);
            }
            out.push(']');
        }
        ExprKind::Paren(inner) => {
            out.push('(');
            write_expr(out, inner);
            out.push(')');
        }
        ExprKind::Range { lo, hi } => {
            write_operand(out, lo, 1);
            out.push_str("..");
            write_operand(out, hi, 1);
        }
    }
}

fn write_args(out: &mut String, args: &[Arg]) {
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        if let Some(name) = &arg.name {
            out.push_str(name.as_str());
            out.push_str(": ");
        }
        write_expr(out, &arg.value);
    }
}

fn write_quantity(out: &mut String, quantity: &Quantity) {
    match quantity.magnitude {
        Magnitude::Integer(n) => out.push_str(&n.to_string()),
        Magnitude::Decimal(decimal) => write_decimal(out, decimal),
        Magnitude::Fraction(fraction) => {
            if fraction.whole > 0 {
                out.push_str(&format!("{}-", fraction.whole));
            }
            out.push_str(&format!("{}/{}", fraction.numerator, fraction.denominator));
        }
    }
    out.push_str(quantity.unit.suffix());
}

fn write_decimal(out: &mut String, decimal: Decimal) {
    let digits = decimal.digits.to_string();
    let scale = usize::from(decimal.scale);
    if scale == 0 {
        out.push_str(&digits);
        return;
    }
    let padded = if digits.len() <= scale {
        format!("{}{digits}", "0".repeat(scale + 1 - digits.len()))
    } else {
        digits
    };
    let point = padded.len() - scale;
    out.push_str(padded.get(..point).unwrap_or("0"));
    out.push('.');
    out.push_str(padded.get(point..).unwrap_or(""));
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::SourceMap;
    use stave_lang::ast::{DesignItem, ItemKind, Let};

    /// Parses `let x = <text>` and prints its value.
    fn round_trip(text: &str) -> String {
        let source = format!("stave 1\n\nlet x = {text}\n");
        let mut sources = SourceMap::new();
        let file = sources.add("t.stave", source.as_str()).unwrap();
        let parse = stave_lang::parse_file(file, &source);
        assert!(!parse.has_errors(), "{text}: {:?}", parse.diagnostics);
        let item = parse.ast.unwrap().items.remove(0);
        let ItemKind::Design(DesignItem::Let(Let { value, .. })) = item.kind else {
            panic!("a let");
        };
        expr_text(&value)
    }

    #[test]
    fn expressions_print_as_written() {
        for text in [
            "19mm",
            "1.5mm",
            "0.05mm",
            "3/4in",
            "1-1/2in",
            "110deg",
            "4",
            "0.5",
            "true",
            "fit",
            "cabinet_width - 2 * overhang",
            "(a + b) * c",
            "a - (b - c)",
            "-reveal / 2",
            "overlay(left: 18mm, right: 18mm, top: 8mm, bottom: 8mm, meeting: -reveal / 2)",
            "side_mount_slide(length: slide_length)",
            "drawer[top].front",
            "foot[*]",
            "drawer_bay[same]",
            "[side_l, side_r]",
            "hw.bar_pull_160",
            "between(slat[0], apron_front) >= 12mm",
            "a && !b",
        ] {
            assert_eq!(round_trip(text), text);
        }
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(quoted("a \"b\" {c}\\"), r#""a \"b\" \{c}\\""#);
        assert_eq!(round_trip(r#""say \"hi\"""#), r#""say \"hi\"""#);
    }
}
