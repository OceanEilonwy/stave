//! Stave's editor features without a user interface: the navigation layer
//! the desktop editor uses today and the language server (`stave-lsp`)
//! will use unchanged (grammar, "Tooling and editors").
//!
//! - [`Analysis`] indexes a loaded project (its own folders, the folders
//!   it imports and the standard library) and answers, at a byte offset:
//!   what a name means and where it is declared ([`Analysis::definition`]),
//!   every use of it with what each use does there ([`Analysis::uses`]:
//!   "declared", "door_r mirrors it", "scene all_open poses it"), what to
//!   show on hover ([`Analysis::hover`]: the doc comment and the
//!   declaration on one line), where the cursor is
//!   ([`Analysis::breadcrumbs`]: `doors.stave › assembly door_l ›
//!   mount`), the colour of every word ([`Analysis::tokens`]), the lenses
//!   above declarations ([`Analysis::lenses`]: "2 uses · show in 3D"),
//!   "Go to file or name" ([`Analysis::search`]) and the fast tier of
//!   diagnostics ([`Analysis::diagnostics`]).
//! - [`Analysis::rename`] gives the text edits that rename an item and
//!   every reference to it, refusing a name that would clash, and
//!   [`Analysis::dependants`] lists the items that name an item, directly
//!   or through others.
//! - [`Buffers`] holds the open documents' text, knows which have unsaved
//!   changes, saves them, and hands their text to the loader and the
//!   checker as an [`Overlay`], so checking reads what the editor shows.
//! - [`Document`] is what an editor opens: a file on disk, or a read-only
//!   file of the standard library (`stave-std:///joinery/dado.stave`).
//!
//! Positions are byte offsets into a file's text; lines and columns are
//! worked out only at the edge ([`Location`]). Nothing here knows about
//! GTK or the language server protocol.
//!
//! # Incrementality
//!
//! An [`Analysis`] is a value: build a new one when a buffer changes. The
//! loader parses every file again (a few milliseconds for the examples);
//! caching parses per file, as the grammar's tooling section asks, is the
//! next step and changes nothing in this interface.
//!
//! ```
//! use std::path::Path;
//! use stave_ide::{Analysis, Document, FilePosition, Overlay};
//!
//! let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/clothing_cabinet");
//! let folder = folder.canonicalize().unwrap();
//! let analysis = Analysis::load(&folder, &Overlay::new()).unwrap();
//! let doors = analysis.file(&Document::File(folder.join("doors.stave"))).unwrap();
//! let text = analysis.text(doors).unwrap();
//! let offset = u32::try_from(text.find("FramePanelDoor").unwrap()).unwrap();
//! let definition = analysis.definition(FilePosition { file: doors, offset }).unwrap();
//! assert_eq!(definition.label(&folder), "std/casework/frame_panel_door.stave:4");
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod analysis;
mod buffers;
mod document;
mod index;
mod rename;
mod tokens;
pub mod words;

use std::path::{Path, PathBuf};

pub use analysis::{Analysis, Crumb, FilePosition, Found, Hover, Lens, Location, Use, Uses};
pub use buffers::{Buffers, OpenError, Overlay, Reload, SaveError};
pub use document::{Document, STD_SCHEME};
pub use index::{Symbol, SymbolId, SymbolKind, UseKind};
pub use rename::{RenameEdit, RenameError};
pub use tokens::{SemanticToken, TokenClass, TokenModifier, TokenModifiers};

/// How deep [`project_files`] looks into subfolders.
const FOLDER_DEPTH: usize = 4;

/// Every `.stave` file in a project folder and its subfolders (hidden
/// folders and `target` left out), sorted by path: what the file sidebar
/// lists.
pub fn project_files(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect(folder, FOLDER_DEPTH, &mut files);
    files.sort();
    files
}

fn collect(folder: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if let Some(depth) = depth.checked_sub(1) {
                collect(&path, depth, files);
            }
        } else if path.extension().is_some_and(|e| e == "stave") {
            files.push(path);
        }
    }
}
