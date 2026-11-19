//! How the window is asked to open: the command line's options, the
//! light or dark style and where the camera starts.

use std::path::PathBuf;

use stave_app::session::View;

use super::edit_demo;
use super::files::Wish;

/// The light or dark style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Follow the system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

impl Theme {
    /// The theme a word names: `system`, `light`, `dark`.
    pub fn from_word(word: &str) -> Option<Theme> {
        match word {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }

    pub(super) fn word(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub(super) fn scheme(self) -> adw::ColorScheme {
        match self {
            Theme::System => adw::ColorScheme::Default,
            Theme::Light => adw::ColorScheme::ForceLight,
            Theme::Dark => adw::ColorScheme::ForceDark,
        }
    }
}

/// How to open the window.
#[derive(Default)]
pub struct Options {
    /// The project folders, each in a window of its own; none shows the
    /// start page.
    pub folders: Vec<PathBuf>,
    /// The scene to start in.
    pub scene: Option<String>,
    /// The item to start with selected, by name.
    pub select: Option<String>,
    /// The joint or hardware to start focused on, by name.
    pub focus: Option<String>,
    /// Focus mode starts looking from above.
    pub plan: bool,
    /// Focus mode starts in 3D, even where it would look from above.
    pub in_3d: bool,
    /// How far along focus mode starts: 0 apart to 1 together, or 0
    /// closed to 1 fully open.
    pub step: Option<f32>,
    /// The problem to start showing (its index in the problems list).
    pub problem: Option<usize>,
    /// The explode amount to start with.
    pub explode: Option<f32>,
    /// The view to start in.
    pub view: View,
    /// Open the editor beside the 3D view.
    pub editor: bool,
    /// Open the finder with this typed (screenshots).
    pub finder: Option<String>,
    /// Start with X-ray on.
    pub xray: bool,
    /// Lay the panels out from this file instead of the user's own.
    pub layout: Option<PathBuf>,
    /// Light, dark or the system's; `None` for the style chosen last
    /// time.
    pub theme: Option<Theme>,
    /// Save the window to this PNG once drawn, then close.
    pub window_screenshot: Option<PathBuf>,
    /// The window's size, in logical pixels; `None` for the size it had
    /// last time.
    pub size: Option<(u32, u32)>,
    /// Start maximised (as it was last time).
    pub maximized: bool,
    /// For the screenshot: hold a second check running, to show the
    /// checking state.
    pub checking: bool,
    /// For the screenshot: press the build button and hold its check
    /// running, to show the building state.
    pub building: bool,
    /// The file to open in the editor, as `doors.stave:9`.
    pub file: Option<String>,
    /// What the editor shows once the names are in (screenshots).
    pub wish: Wish,
    /// Where the camera starts (screenshots of a close view).
    pub camera: StartCamera,
    /// Show the editing menu or card from the contract's fixture.
    pub edit_fixture: Option<edit_demo::Showcase>,
    /// Editing steps to run once the design is drawn (screenshots).
    pub edit_script: Option<PathBuf>,
    /// Open the new project dialog.
    pub new_project: bool,
}

/// Where the 3D view's camera starts, from the command line.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StartCamera {
    /// Degrees about the up axis from the default view.
    pub turn: f32,
    /// Degrees above the horizontal, instead of the default.
    pub pitch: Option<f32>,
    /// Frame the selection rather than everything.
    pub frame_selection: bool,
    /// Move this many times closer once framed.
    pub zoom: Option<f32>,
}
