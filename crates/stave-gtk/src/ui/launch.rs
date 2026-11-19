//! What one launch of Stave hands to the Stave that carries it out.
//!
//! Stave runs as one process for every project window (GApplication's
//! single instance). Each launch is turned into a short command line that
//! GApplication carries to that process (`HANDLES_COMMAND_LINE`): to this
//! one when it is the first Stave, or over D-Bus to the Stave already
//! running. There it is read back into a [`Launch`] and given to the
//! application's component as a message.
//!
//! Only what makes sense for a Stave that is already running travels:
//!
//! - the folders to open (each opens a window, or brings forward the one
//!   already showing it);
//! - `--theme`, which is app-wide (libadwaita's style manager), so a
//!   second `stave-gtk --theme dark` turns every open window dark, as the
//!   style menu would. Like `--theme` at startup it is not remembered:
//!   the style chosen in the menu still comes back next time.
//!
//! Everything else stays with the launch that was given it. `--screenshot`
//! never starts GTK, and `--window-screenshot` always runs a Stave of its
//! own. The rest (`--scene`, `--select`, `--view`, `--editor`, `--file`,
//! the camera, `--layout`, `--size` and the like) say how a fresh window
//! starts; a running Stave may already show that folder, and changing a
//! window people are working in from another terminal would surprise
//! them, so they are left out and the launch says so (see `main.rs`).

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use gtk::gio;
use gtk::prelude::*;

use super::Theme;

/// The program name the carried command line starts with.
const PROGRAM: &str = "stave-gtk";

/// The option the style travels in, with its word joined on
/// (`--theme=dark`) so it is one argument.
const THEME_OPTION: &str = "--theme=";

/// What one launch asks of the running Stave.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Launch {
    /// The style to show from now on, app-wide; `None` leaves it.
    pub(super) theme: Option<Theme>,
    /// The project folders to open, already absolute; none shows the
    /// window in front, or the start page.
    pub(super) folders: Vec<PathBuf>,
}

impl Launch {
    /// The command line GApplication carries: the program, the style if
    /// any, then each folder as a `file://` URI (which never starts with a
    /// dash, so no folder is read as an option, and survives any bytes in
    /// its name).
    pub(super) fn to_arguments(&self) -> Vec<String> {
        let mut arguments = vec![PROGRAM.to_owned()];
        if let Some(theme) = self.theme {
            arguments.push(format!("{THEME_OPTION}{}", theme.word()));
        }
        arguments.extend(
            self.folders
                .iter()
                .map(|folder| gio::File::for_path(folder).uri().to_string()),
        );
        arguments
    }

    /// Reads back a command line [`Launch::to_arguments`] wrote. A `--`
    /// on its own is passed over.
    pub(super) fn from_arguments(arguments: &[OsString]) -> Result<Launch> {
        let mut launch = Launch::default();
        for argument in arguments.iter().skip(1) {
            let text = argument
                .to_str()
                .ok_or_else(|| anyhow!("{argument:?} is not a folder's address"))?;
            if text == "--" {
                continue;
            }
            if let Some(word) = text.strip_prefix(THEME_OPTION) {
                launch.theme =
                    Some(Theme::from_word(word).ok_or_else(|| {
                        anyhow!("the style is system, light or dark, not {word}")
                    })?);
            } else if text.starts_with('-') {
                bail!("a running Stave doesn't take {text}");
            } else {
                let folder = gio::File::for_uri(text)
                    .path()
                    .ok_or_else(|| anyhow!("{text} is not a folder on this computer"))?;
                launch.folders.push(folder);
            }
        }
        Ok(launch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_launch_round_trips_through_its_command_line() {
        for launch in [
            Launch::default(),
            Launch {
                theme: Some(Theme::Dark),
                folders: Vec::new(),
            },
            Launch {
                theme: Some(Theme::System),
                folders: vec![
                    PathBuf::from("/home/someone/projects/wall shelf"),
                    PathBuf::from("/tmp/--theme=light"),
                    PathBuf::from("/tmp/caf\u{e9}%20table"),
                ],
            },
            Launch {
                theme: None,
                folders: vec![PathBuf::from("/tmp/stool")],
            },
        ] {
            let carried: Vec<OsString> = launch
                .to_arguments()
                .into_iter()
                .map(OsString::from)
                .collect();
            assert_eq!(carried.first(), Some(&OsString::from(PROGRAM)));
            assert_eq!(Launch::from_arguments(&carried).unwrap(), launch);
        }
    }

    #[test]
    fn the_style_travels_as_one_argument_before_the_folders() {
        let launch = Launch {
            theme: Some(Theme::Dark),
            folders: vec![PathBuf::from("/tmp/stool")],
        };
        assert_eq!(
            launch.to_arguments(),
            ["stave-gtk", "--theme=dark", "file:///tmp/stool"]
        );
    }

    #[test]
    fn a_command_line_it_did_not_write_is_refused() {
        for words in [
            &["stave-gtk", "--theme=sepia"][..],
            &["stave-gtk", "--scene=all_open"],
            &["stave-gtk", "--theme", "dark"],
            &["stave-gtk", "https://example.com/shelf"],
        ] {
            assert!(
                Launch::from_arguments(&arguments(words)).is_err(),
                "{words:?} was taken"
            );
        }
        assert_eq!(
            Launch::from_arguments(&arguments(&["stave-gtk", "--", "file:///tmp/a"])).unwrap(),
            Launch {
                theme: None,
                folders: vec![PathBuf::from("/tmp/a")],
            }
        );
    }
}
