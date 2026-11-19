//! Translation of the interface's words.
//!
//! Every sentence the window shows goes through [`tr`] (or [`ntr`] when it
//! counts something), looked up in the `stave` text domain with gettext.
//! With no translation installed, or before [`init`] runs (the command
//! line never calls it), the English text comes back unchanged, so tests
//! and the command line see exactly the words written in the code.
//!
//! Rules for the text handed to these functions, so a translator can work
//! with it:
//!
//! - Whole sentences, never pieces glued together: a sentence with values
//!   in it is one template with named holes (`"{count} parts hidden"`),
//!   filled with [`fill_in`] after the lookup, so a translation can move the
//!   holes where its grammar needs them.
//! - A count chooses its words with [`ntr`], never with an `if`, because
//!   many languages have more than two plural forms.
//! - The arguments are string literals, so `xgettext` can collect them:
//!   `po/update-pot` lists the keywords (`tr`, `ntr:1,2`, `trc:1c,2`, `tr_noop`).
//!
//! The diagnostics the core crates write (`stave-lang` to `stave-check`)
//! are not translated: they are the compiler's messages, shared with the
//! command line and its snapshot tests, and stay in English.

use std::path::Path;

/// The gettext text domain: the name of the `.mo` file a translation is
/// installed as (`<locale dir>/<lang>/LC_MESSAGES/stave.mo`).
pub const DOMAIN: &str = "stave";

/// Bind the `stave` text domain to `locale_dir` (when given; otherwise
/// gettext's default directory), so [`tr`] finds installed translations.
/// Called once, at the window's start-up. The language itself comes from
/// the program's locale, which GTK sets from the environment when it
/// starts (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`). A failure only
/// leaves the interface in English, so it is reported, not fatal.
///
/// # Errors
///
/// When the C library refuses the text domain or its directory.
pub fn init(locale_dir: Option<&Path>) -> std::io::Result<()> {
    if let Some(directory) = locale_dir {
        gettextrs::bindtextdomain(DOMAIN, directory)?;
    }
    gettextrs::bind_textdomain_codeset(DOMAIN, "UTF-8")?;
    gettextrs::textdomain(DOMAIN)?;
    Ok(())
}

/// The translation of `message`, or `message` itself when there is none.
#[must_use]
pub fn tr(message: &str) -> String {
    gettextrs::dgettext(DOMAIN, message)
}

/// The translation of `singular` or `plural`, chosen for `count` by the
/// language's own plural rule (English: `singular` for one, `plural`
/// otherwise). The chosen template usually holds `{count}`; fill it with
/// [`fill_in`].
#[must_use]
pub fn ntr(singular: &str, plural: &str, count: usize) -> String {
    // gettext counts in `u32`; past that, every language uses its plural.
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    gettextrs::dngettext(DOMAIN, singular, plural, count)
}

/// The translation of `message` in `context`: for a short word that means
/// different things in different places (`trc("verb", "Open")`), so a
/// translator can give each its own translation. Without one, `message`.
#[must_use]
pub fn trc(context: &str, message: &str) -> String {
    let key = format!("{context}\u{4}{message}");
    let translated = gettextrs::dgettext(DOMAIN, key.as_str());
    if translated == key {
        message.to_owned()
    } else {
        translated
    }
}

/// `message` itself, marked for `xgettext` to collect: for text written
/// into a table or a constant, which is translated with [`tr`] where it is
/// shown.
#[must_use]
pub const fn tr_noop(message: &'static str) -> &'static str {
    message
}

/// `template` with each `{name}` replaced by the value paired with `name`.
/// Holes with no value, and braces that are not a hole, stay as written;
/// the values themselves are never searched for holes.
#[must_use]
pub fn fill_in(template: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from_brace) = rest.split_at(open);
        out.push_str(before);
        let after_brace = from_brace.get(1..).unwrap_or_default();
        let hole = after_brace
            .find('}')
            .map(|close| (after_brace.get(..close).unwrap_or_default(), close));
        match hole.and_then(|(name, close)| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (value, close))
        }) {
            Some((value, close)) => {
                let _ = write!(out, "{value}");
                rest = after_brace.get(close + 1..).unwrap_or_default();
            }
            None => {
                out.push('{');
                rest = after_brace;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untranslated_text_comes_back_as_written() {
        assert_eq!(tr("Open Project…"), "Open Project…");
        assert_eq!(trc("verb", "Open"), "Open");
    }

    #[test]
    fn plurals_follow_the_count() {
        assert_eq!(ntr("{count} part", "{count} parts", 1), "{count} part");
        assert_eq!(ntr("{count} part", "{count} parts", 0), "{count} parts");
        assert_eq!(ntr("{count} part", "{count} parts", 7), "{count} parts");
    }

    #[test]
    fn fill_replaces_named_holes() {
        let words = "door_l";
        assert_eq!(
            fill_in(
                "Off screen: {words} · {more} more",
                &[("words", &words), ("more", &3)]
            ),
            "Off screen: door_l · 3 more"
        );
    }

    #[test]
    fn fill_leaves_unknown_holes_and_values_alone() {
        let value = "{other}";
        assert_eq!(
            fill_in("{a} {missing} {", &[("a", &value), ("other", &1)]),
            "{other} {missing} {"
        );
    }
}
