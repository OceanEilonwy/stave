//! "Export 3D Model…": the 3D view as it is posed (pulled apart,
//! opened) saved as a binary glTF file, the same file `stave export`
//! writes.

use crate::i18n::{fill_in, tr};
use std::path::Path;

use stave_model::solved::SolvedDesign;
use stave_scene::{Posed, SceneGraph};

/// Why the view couldn't be saved.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The glTF couldn't be made.
    #[error("{}", self.text())]
    Export(#[from] stave_scene::export::ExportError),
    /// The file couldn't be written.
    #[error("{}", self.text())]
    Write {
        /// Where.
        path: String,
        /// Why.
        source: std::io::Error,
    },
}

impl SaveError {
    /// The error in the window's words (translated).
    fn text(&self) -> String {
        match self {
            SaveError::Export(v0) => {
                fill_in(&tr("can't make the glTF file: {value}"), &[("value", v0)])
            }
            SaveError::Write { path, source } => fill_in(
                &tr("can't write '{path}': {source}"),
                &[("path", path), ("source", source)],
            ),
        }
    }
}

/// Saves the view as posed (pulled apart, opened) as a binary glTF file,
/// as `stave export` does.
pub fn save_glb(graph: &SceneGraph, posed: &Posed, path: &Path) -> Result<(), SaveError> {
    let bytes = graph.to_glb(posed)?;
    std::fs::write(path, bytes).map_err(|source| SaveError::Write {
        path: path.display().to_string(),
        source,
    })
}

/// The file name the save dialog offers: the project's title in lower
/// case with every character that is not a letter or a digit made an
/// underscore, then `_view.glb` ("Hall Bench" becomes
/// `hall_bench_view.glb`); `design_view.glb` when nothing is shown.
pub fn default_file_name(design: Option<&SolvedDesign>) -> String {
    let title = design.map_or("design", |design| design.project.title.as_str());
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!("{slug}_view.glb")
}
