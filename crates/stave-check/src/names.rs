//! How values are written in messages.

use stave_lang::units::Length;
use stave_model::solved::PoseValue;

/// A length as messages show it: millimetres, rounded to the micrometre,
/// trailing zeros trimmed (`3mm`, `0.5mm`).
pub(crate) fn length_text(length: Length) -> String {
    let micrometres = (length.nm() + 500).div_euclid(1000);
    Length::from_nm(micrometres.saturating_mul(1000)).to_string()
}

/// A scene's pose value: `open`, `closed`, `open 90deg`, `out 200mm`.
pub(crate) fn pose_text(value: PoseValue) -> String {
    match value {
        PoseValue::Open => "open".to_owned(),
        PoseValue::Closed => "closed".to_owned(),
        PoseValue::Angle(angle) => format!("open {angle}"),
        PoseValue::Travel(length) => format!("out {}", length_text(length)),
    }
}

/// `a`, `a and b`, `a, b and c`.
pub(crate) fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The same as [`list`]; for conditions (`door is open and drawer is out
/// 200mm`).
pub(crate) fn and_list(items: &[String]) -> String {
    list(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_round_to_the_micrometre() {
        assert_eq!(length_text(Length::from_nm(2_999_999)), "3mm");
        assert_eq!(length_text(Length::from_nm(500_000)), "0.5mm");
        assert_eq!(length_text(Length::from_nm(12_345_678)), "12.346mm");
    }

    #[test]
    fn lists_read_as_english() {
        let items = ["a".to_owned(), "b".to_owned(), "c".to_owned()];
        assert_eq!(list(&items[..1]), "a");
        assert_eq!(list(&items[..2]), "a and b");
        assert_eq!(list(&items), "a, b and c");
    }
}
