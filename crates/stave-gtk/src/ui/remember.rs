//! What the window remembers from one run to the next: its size, whether
//! it was maximised, and the style chosen in the menu. It lives beside
//! the panel layout, in `~/.config/stave/window`, as a GLib key file:
//!
//! ```text
//! [window]
//! width=1200
//! height=800
//! maximized=false
//! style=dark
//! ```
//!
//! A key file rather than GSettings: GSettings needs its schema compiled
//! and installed before the first run, and Stave also runs straight from
//! `cargo run`. Screenshots neither read nor write it.

use std::path::PathBuf;

use anyhow::{Context as _, Result};
use gtk::glib;

use super::Theme;

/// The window's size before anything is remembered, in logical pixels:
/// small enough for a laptop screen at 125% scaling.
pub(super) const DEFAULT_SIZE: (i32, i32) = (1200, 800);

/// The smallest size remembered; anything smaller is taken as a mistake.
const SMALLEST: i32 = 360;

/// The key file's one group.
const GROUP: &str = "window";

/// What is remembered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Remembered {
    /// The window's size when not maximised, in logical pixels.
    pub(super) size: (i32, i32),
    /// The window was maximised when it closed.
    pub(super) maximized: bool,
    /// The style chosen in the menu.
    pub(super) theme: Theme,
}

impl Default for Remembered {
    fn default() -> Self {
        Remembered {
            size: DEFAULT_SIZE,
            maximized: false,
            theme: Theme::System,
        }
    }
}

impl Remembered {
    /// Reads what a key file's text says; whatever is missing or wrong
    /// keeps its default.
    pub(super) fn from_text(text: &str) -> Remembered {
        let mut remembered = Remembered::default();
        let file = glib::KeyFile::new();
        if file.load_from_data(text, glib::KeyFileFlags::NONE).is_err() {
            return remembered;
        }
        if let (Ok(width), Ok(height)) =
            (file.integer(GROUP, "width"), file.integer(GROUP, "height"))
            && width >= SMALLEST
            && height >= SMALLEST
        {
            remembered.size = (width, height);
        }
        if let Ok(maximized) = file.boolean(GROUP, "maximized") {
            remembered.maximized = maximized;
        }
        if let Some(theme) = file
            .string(GROUP, "style")
            .ok()
            .and_then(|word| Theme::from_word(word.as_str()))
        {
            remembered.theme = theme;
        }
        remembered
    }

    /// The key file's text.
    pub(super) fn to_text(self) -> String {
        let file = glib::KeyFile::new();
        file.set_integer(GROUP, "width", self.size.0);
        file.set_integer(GROUP, "height", self.size.1);
        file.set_boolean(GROUP, "maximized", self.maximized);
        file.set_string(GROUP, "style", self.theme.word());
        file.to_data().to_string()
    }

    /// What was remembered last time, or the defaults.
    pub(super) fn load() -> Remembered {
        std::fs::read_to_string(path())
            .map(|text| Remembered::from_text(&text))
            .unwrap_or_default()
    }

    /// Writes it for next time.
    pub(super) fn save(self) -> Result<()> {
        let path = path();
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)
                .with_context(|| format!("can't make the folder '{}'", folder.display()))?;
        }
        std::fs::write(&path, self.to_text())
            .with_context(|| format!("can't write '{}'", path.display()))
    }
}

/// `~/.config/stave/window`.
fn path() -> PathBuf {
    glib::user_config_dir().join("stave").join("window")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() {
        let remembered = Remembered {
            size: (1500, 950),
            maximized: true,
            theme: Theme::Dark,
        };
        assert_eq!(Remembered::from_text(&remembered.to_text()), remembered);
    }

    #[test]
    fn nothing_remembered_gives_a_modest_window() {
        assert_eq!(Remembered::from_text(""), Remembered::default());
        assert_eq!(Remembered::default().size, (1200, 800));
    }

    #[test]
    fn a_broken_file_keeps_the_defaults() {
        let text = "[window]\nwidth=12\nheight=banana\nstyle=purple\nmaximized=perhaps\n";
        assert_eq!(Remembered::from_text(text), Remembered::default());
        assert_eq!(
            Remembered::from_text("not a key file at all"),
            Remembered::default()
        );
    }
}
