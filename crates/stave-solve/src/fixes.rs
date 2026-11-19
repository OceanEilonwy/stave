//! Fixes the solver offers (grammar, "Fixes"): patches that make one of
//! its diagnostics go away.

use stave_lang::ast::Span;
use stave_lang::units::Rational;
use stave_lang::{FileId, Fix};
use stave_model::value::Value;

/// Where a value is written, and the path a patch sets it by (`None`
/// when no patch reaches it: it is written in a library, or inside an
/// expression).
pub(crate) struct Written {
    /// The file.
    pub(crate) file: FileId,
    /// The value as written.
    pub(crate) span: Span,
    /// The path a `set` names: `runner.length`, `block.height`, `width`.
    pub(crate) path: Option<String>,
}

/// A value written as source text that reads back as exactly the same
/// value: `300mm`, `95deg`, `3`, `1/3`. `None` for a value that isn't a
/// quantity, or a length or angle finer than the units can write.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "picks out only the variants it names: any other, a new one included, is not one of them"
)]
pub(crate) fn value_source(value: &Value) -> Option<String> {
    match value {
        Value::Length(length) => length.to_length().map(|length| length.to_string()),
        Value::Angle(angle) => angle.to_angle().map(|angle| angle.to_string()),
        Value::Count(number) | Value::Ratio(number) => Some(number.to_string()),
        _ => None,
    }
}

/// A quantity's exact amount, for measuring how far apart two values are.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "picks out only the variants it names: any other, a new one included, is not one of them"
)]
fn amount(value: &Value) -> Option<Rational> {
    match value {
        Value::Length(length) => Some(length.value()),
        Value::Angle(angle) => Some(angle.value()),
        Value::Count(number) | Value::Ratio(number) => Some(*number),
        _ => None,
    }
}

/// How far apart two quantities are; `None` when either isn't one.
fn distance(a: &Value, b: &Value) -> Option<Rational> {
    let difference = amount(a)?.checked_sub(amount(b)?)?;
    if difference.is_negative() {
        difference.checked_neg()
    } else {
        Some(difference)
    }
}

/// The allowed value nearest to `value`, of `allowed` in their order: the
/// first of two that are as near.
pub(crate) fn nearest<'v>(value: &Value, allowed: &'v [Value]) -> Option<&'v Value> {
    let mut best: Option<(Rational, &Value)> = None;
    for candidate in allowed {
        let Some(far) = distance(value, candidate) else {
            continue;
        };
        if best.is_none_or(|(nearest, _)| far < nearest) {
            best = Some((far, candidate));
        }
    }
    best.map(|(_, candidate)| candidate)
}

/// The fix that sets `path` to `value`: `patch "Use 300mm" { set
/// runner.length = 300mm }`. `None` when the value can't be written.
pub(crate) fn set_to(path: &str, value: &Value) -> Option<Fix> {
    let text = value_source(value)?;
    Fix::new(format!("Use {text}"), &format!("set {path} = {text}")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use stave_lang::units::Length;

    fn mm(value: i64) -> Value {
        Value::Length(Length::from_mm(value).to_exact())
    }

    #[test]
    fn the_nearest_value_wins_and_the_first_of_a_tie() {
        let allowed = [mm(250), mm(300), mm(350)];
        assert_eq!(nearest(&mm(320), &allowed), Some(&mm(300)));
        assert_eq!(nearest(&mm(330), &allowed), Some(&mm(350)));
        assert_eq!(nearest(&mm(325), &allowed), Some(&mm(300)));
        assert_eq!(nearest(&mm(10), &allowed), Some(&mm(250)));
        assert_eq!(nearest(&mm(10), &[]), None);
    }

    #[test]
    fn a_fix_sets_the_path_to_the_value() {
        let fix = set_to("runner.length", &mm(300)).unwrap();
        assert_eq!(
            fix.to_string(),
            "patch \"Use 300mm\" { set runner.length = 300mm }"
        );
        let third = Value::Ratio(Rational::new(1, 3).unwrap());
        assert_eq!(value_source(&third).as_deref(), Some("1/3"));
    }
}
