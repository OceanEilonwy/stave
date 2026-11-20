//! Stave's language server: the editor features of [`stave_ide`] and the
//! checks of [`stave_app`] over the language server protocol, so `.stave`
//! files can be edited in any editor (grammar, "Tooling and editors").
//!
//! The binary is `stave-lsp`. It takes no arguments and speaks the
//! protocol over standard input and output; an editor starts it for files
//! ending in `.stave`.
//!
//! # What it answers
//!
//! - **Documents.** `didOpen`, `didChange` (incremental) and `didClose`
//!   keep the editor's text in [`stave_ide::Buffers`]; every change loads
//!   the file's project again through the buffers' [`stave_ide::Overlay`],
//!   so unsaved text is what is checked. A file's project is the nearest
//!   folder at or above it whose files declare a `project` (so a library
//!   folder inside a project, like `hardware/`, is part of it), or the
//!   file's own folder.
//! - **Diagnostics, in two tiers** (grammar, "Fast and slow diagnostics").
//!   On every change the loader's diagnostics (syntax, imports, module
//!   headers) from [`stave_ide::Analysis::diagnostics`] are published at
//!   once. When typing has paused for [`Config::slow_delay`], the whole
//!   pipeline ([`stave_app::open_with`]) runs in a background thread on a
//!   snapshot of the buffers, and its diagnostics are published with the
//!   fast ones. A change drops the slow diagnostics of the text before it
//!   and starts the wait again; a check still running when its text
//!   changes is not interrupted (z3 can't be stopped midway), but its
//!   result is dropped and the next check starts when it ends.
//! - **Navigation:** definition, references, hover, rename (with
//!   `prepareRename`; a name declared outside the project is refused).
//! - **Semantic tokens** (full), with the legend below.
//! - **Completion** and **signature help** from
//!   [`stave_ide::Analysis::completions`] and
//!   [`stave_ide::Analysis::signature_help`].
//! - **Code lens:** "2 uses" above each part, assembly, piece of hardware,
//!   component and kind, running the client command `stave.showUses`
//!   (arguments: the document URI and the name's position), and "show in
//!   3D" on what the 3D view can show, running `stave.showIn3d` (argument:
//!   the item's name). Editors that don't know the commands show the
//!   lenses as text.
//! - **The standard library** is served read-only as
//!   `stave-std:///joinery/dado.stave` documents, so go-to-definition
//!   reaches `dado` and every built-in kind. An editor that can't open a
//!   `stave-std:` URI itself asks for its text with the request
//!   `stave/standardDocument`, params `{"uri": "stave-std:///…"}`, result
//!   `{"text": "…"}`. Changes sent for such a document are ignored.
//!
//! Positions are converted at the edge, in [`line_index`]: UTF-16 code
//! units unless the client offers UTF-8.
//!
//! # Semantic token legend
//!
//! Token types, in legend order (the index is the type's number):
//!
//! | # | Type | What | Stave's class |
//! |---|---|---|---|
//! | 0 | `keyword` | keywords, contextual words in place | `Keyword` |
//! | 1 | `comment` | comments; doc comments carry `documentation` | `Comment`, `DocComment` |
//! | 2 | `string` | strings | `Text` |
//! | 3 | `number` | numbers, and a quantity's number (`18` of `18mm`) | `Number` |
//! | 4 | `operator` | brackets, operators, separators | `Punctuation` |
//! | 5 | `function` | fixed call heads, built-in functions (`flush`, `mid`) | `Function` |
//! | 6 | `type` | type names (`Region`), components and kinds named as types | `Type` |
//! | 7 | `namespace` | modules (`casework`, `hw`) | `Module`, `Item(Module)` |
//! | 8 | `enumMember` | words of a closed list (`up`, `open`, `plywood`) | `EnumWord` |
//! | 9 | `property` | fields, named arguments, members after `.` | `Field` |
//! | 10 | `variable` | a name that resolves to nothing (with `unresolved`) | `Plain` |
//! | 11 | `unit` | a quantity's unit (`mm` of `18mm`, `deg`) | `Unit` |
//! | 12–29 | `component`, `kind`, `recipe`, `stock`, `profile`, `scene`, `param`, `let`, `region`, `part`, `joint`, `hardware`, `assembly`, `require`, `role`, `madePart`, `member`, `label` | a name of an item of that kind | `Item(..)` |
//!
//! Types 11 to 29 are Stave's own; an editor maps them to colours (the VS
//! Code extension declares them with a standard super type: `unit` →
//! `keyword` or `number`, the item kinds → `variable`, `component` and
//! `kind` → `type`). Text with no colour (a name that means nothing and
//! has no modifier, an unreadable character) is not sent.
//!
//! Token modifiers, in legend order (the bit is `1 << index`):
//!
//! | # | Modifier | What |
//! |---|---|---|
//! | 0 | `declaration` | the name an item is declared with |
//! | 1 | `reference` | a name that means an item declared elsewhere (underlined in the GTK editor) |
//! | 2 | `defaultLibrary` | a name of an item from the standard library |
//! | 3 | `unresolved` | a word where a name goes that means nothing |
//! | 4 | `solvedParam` | a `solved param`, declared or named |
//! | 5 | `documentation` | a doc comment (`///`) |
//!
//! Every token is on one line: a comment over several lines is sent as one
//! token per line.
//!
//! # Why `lsp-server` and `lsp-types`
//!
//! The protocol layer is rust-analyzer's: `lsp-server` (the message loop
//! and transport, maintained in the rust-analyzer repository) and
//! `lsp-types` (the protocol's types). Both are mature and widely used,
//! synchronous, and need no async runtime: the analysis is synchronous and
//! fast, and the one slow job (the background check) is a plain thread.
//! `Connection::memory` gives the integration test an in-memory client.
//! The async alternatives (`tower-lsp-server`) would add an executor for
//! nothing Stave needs.
//!
//! # Simplifications
//!
//! - Names, types and placement (layers 2 and 3) come with the background
//!   check rather than on every keystroke: today only the full pipeline
//!   reports them. The editor's own queries are incremental (step 4 of
//!   the plan); the checker's layers are not yet.
//! - Every change loads the project again through the project's
//!   [`stave_ide::AnalysisHost`], which parses and resolves again only
//!   what the change touched (well under a millisecond for an edit on
//!   the examples); requests are answered one at a time on the main loop,
//!   so `$/cancelRequest` has nothing to cancel.
//! - Files changed on disk by another program are read when the next
//!   change to an open file of the project loads it again.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod convert;
mod legend;
pub mod line_index;
mod server;
mod uri;

use std::time::Duration;

use lsp_server::{Connection, ProtocolError};

pub use convert::{DIAGNOSTIC_SOURCE, SHOW_IN_3D_COMMAND, SHOW_USES_COMMAND};
pub use uri::{document_uri, uri_document};

/// The request that gives a standard library document's text.
pub const STANDARD_DOCUMENT_REQUEST: &str = "stave/standardDocument";

/// How the server behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// How long typing must pause before the slow checks run.
    pub slow_delay: Duration,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            slow_delay: Duration::from_millis(400),
        }
    }
}

/// Why the server stopped with an error.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// The connection broke or the client broke the protocol.
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    /// The client's `initialize` request could not be read.
    #[error("can't read the initialize request: {0}")]
    Initialize(String),
    /// Reading standard input or writing standard output failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The client went away while the server was sending to it.
    #[error("the client disconnected")]
    Disconnected,
    /// The client sent `exit` without `shutdown` first; the protocol asks
    /// the server to stop with an error exit code then.
    #[error("the client exited without shutting the server down")]
    ExitWithoutShutdown,
}

/// Serves one client on a connection until it exits: the `initialize`
/// handshake, then requests and notifications. Returns when the client
/// sends `exit` after `shutdown` or the connection closes.
///
/// # Errors
///
/// A broken connection or protocol, an unreadable `initialize`
/// request, or `exit` without `shutdown`; see [`ServerError`].
pub fn run(connection: &Connection, config: Config) -> Result<(), ServerError> {
    server::run(connection, config)
}

/// Serves one client over standard input and output, as the `stave-lsp`
/// binary does.
///
/// # Errors
///
/// As [`run`], and a failure of the input and output threads.
pub fn serve_stdio(config: Config) -> Result<(), ServerError> {
    let (connection, io_threads) = Connection::stdio();
    run(&connection, config)?;
    drop(connection);
    io_threads.join()?;
    Ok(())
}
