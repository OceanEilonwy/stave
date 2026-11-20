//! Stave's editor features without a user interface: the navigation layer
//! the desktop editor and the language server (`stave-lsp`) both use
//! (grammar, "Tooling and editors").
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
//! - [`Analysis::outline`] and [`Analysis::folds`] give a file's
//!   declarations and the blocks inside them, and the ranges an editor
//!   can fold; [`Analysis::help_fixes`] turns a fast diagnostic's help
//!   line into a text edit when it says exactly what to write.
//! - [`WorkedValues`] holds the values the background check worked out
//!   (`stave_app::editor_check`), shown as inlay hints
//!   ([`WorkedValues::inlay_hints`]: `length: fit` shows `= 844mm`) and
//!   on hover ([`Analysis::hover_with_values`]) while the text they were
//!   worked out for is unchanged.
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
//! An [`Analysis`] is a value: build a new one when a buffer changes.
//! [`Analysis::load`] works everything out; an editor keeps an
//! [`AnalysisHost`] per project instead, and [`AnalysisHost::analyse`]
//! gives the same analysis reusing every result the change did not
//! touch, as the grammar's tooling section asks. File texts are the
//! input; a file's parse, its items, each module's names, the resolution
//! of each file, the index, each file's tokens, the completions at a
//! position and the fast diagnostics are derived queries, each kept until
//! one of its inputs changes. An edit inside one item parses that file
//! again and resolves again only the files whose lookups now find
//! something else ([`QueryStats`] says which).
//!
//! ## A small hand-written query layer, not `salsa`
//!
//! rust-analyzer's `salsa` was weighed and not taken. It would bring a
//! dozen crates (hash maps, lock-free collections, its macros and their
//! proc-macro dependencies) into a crate that today needs only `rowan`,
//! keep its memos in hash tables whose order is not part of the result,
//! and want every query written as a tracked function over interned
//! inputs. Stave's queries are few and their dependencies are known ahead
//! of time, so a few hundred lines do the same with only `BTreeMap`s,
//! one thread and no new dependency:
//!
//! | Query | Input | Kept while |
//! |---|---|---|
//! | parse | a file's id and text | both are the same ([`stave_modules::Parses`]) |
//! | items | the file's tree and module | the file was not parsed again |
//! | module names | its files' items | none of its files' items changed |
//! | assemblies | every file's items and names | worked out again on every change (a few lookups) |
//! | resolution | the file's tree, its imports, the names it looked up | the tree is the same and every lookup it made finds the same thing |
//! | index | every file's items and resolution | put together from the kept parts on every change; every symbol's uses are worked out the first time they are asked for |
//! | tokens | the file's tree and resolution | the resolution is kept |
//! | completions | the analysis and a position | the analysis lives |
//! | fast diagnostics | the parses and the loader's checks | the analysis lives |
//!
//! The layout of the project (its modules, their files and the files'
//! ids) is the one input everything depends on: when it changes (a file
//! added, an import changed), everything but the parses is worked out
//! again.
//!
//! A resolution records every lookup it makes (a name in a module, in a
//! component or kind, among a file's imports, what a symbol is, which
//! component an assembly is an instance of) and what it found. A kept
//! resolution is reused only when every recorded lookup, made again on
//! the new names, finds the same thing; this is `salsa`'s early cut-off,
//! checked on values, so the result is exactly what working everything
//! out again gives. Most kept resolutions need not even look: each also
//! notes which files' declarations and which modules' names it read, and
//! one that read nothing that changed is kept as it is. Because every result is a pure function of its
//! inputs and every collection is ordered, results are the same run to
//! run, and the same whether reused or worked out again
//! (`tests/incremental.rs`).
//!
//! On `examples/clothing_cabinet` and the standard library (release
//! build, 2026-11-20, a field typed letter by letter into a part; `cargo
//! test --release -p stave-ide --test incremental -- --ignored
//! --nocapture` prints them again):
//!
//! | Per keystroke | Time |
//! |---|---|
//! | [`Analysis::load`], everything worked out | 7.7 ms |
//! | [`AnalysisHost::analyse`] | 0.65 ms |
//! | [`Analysis::tokens`] of the edited file | 0.08 ms |
//! | [`Analysis::completions`] at the cursor | 0.08 ms |
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
mod completion;
mod document;
mod fixes;
mod host;
mod index;
mod outline;
mod rename;
mod tokens;
mod values;
pub mod words;

use std::path::{Path, PathBuf};

pub use analysis::{Analysis, Crumb, FilePosition, Found, Hover, Lens, Location, Use, Uses};
pub use buffers::{Buffers, MergeText, OpenError, Overlay, Reload, SaveError, Saved};
pub use completion::{
    Completion, CompletionKind, ParameterKind, SignatureHelp, SignatureParameter,
};
pub use document::{Document, STD_SCHEME};
pub use fixes::HelpFix;
pub use host::{AnalysisHost, QueryStats};
pub use index::{Symbol, SymbolId, SymbolKind, UseKind};
pub use outline::{Fold, FoldKind, OutlineEntry, OutlineKind};
pub use rename::{RenameEdit, RenameError};
pub use tokens::{SemanticToken, TokenClass, TokenModifier, TokenModifiers};
pub use values::{InlayHint, ValueKind, ValueNote, WorkedValues};

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
