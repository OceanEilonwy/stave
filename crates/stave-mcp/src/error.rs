//! What can go wrong in the server, each in plain words an AI app can
//! pass on to the person using it.

use std::path::PathBuf;

/// Why a tool or the server itself couldn't do what was asked.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    /// A tool that works on the open project was called before
    /// `open_project`.
    #[error("No project is open. Call open_project with the project's folder first.")]
    NoProject,
    /// A project folder was not given as a full path.
    #[error("'{0}' is not a full path. Give the folder's full path, starting with /.")]
    NotAbsolute(String),
    /// A project folder was given with a `..` step in it.
    #[error("'{0}' steps out of a folder with '..'. Give the folder's full path without '..'.")]
    ParentStep(String),
    /// `new_project` was given a symbolic link rather than a folder.
    #[error("'{}' is a link to another place. Give the real folder's path.", .0.display())]
    Link(PathBuf),
    /// A path leads outside the project folder: `..`, an absolute path,
    /// or a symbolic link out of it.
    #[error(
        "'{0}' is outside the project folder. Only files inside the project can be read or changed."
    )]
    OutsideProject(String),
    /// There is no folder at the path given.
    #[error("There is no folder at '{}'.", .0.display())]
    NotAFolder(PathBuf),
    /// The folder holds no `.stave` file.
    #[error("'{}' holds no .stave files, so it isn't a Stave project.", .0.display())]
    NotAProject(PathBuf),
    /// `new_project` was given a folder that already holds files.
    #[error(
        "'{}' isn't empty. A new project needs an empty folder, or one that doesn't exist yet.",
        .0.display()
    )]
    FolderNotEmpty(PathBuf),
    /// The project's files couldn't be loaded.
    #[error("Stave can't read the project in '{}': {source}", .folder.display())]
    Load {
        /// The project folder.
        folder: PathBuf,
        /// What the loader said.
        source: stave_app::LoadError,
    },
    /// A file or folder couldn't be read or written.
    #[error("Stave can't use '{}': {source}", .path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What the system said.
        source: std::io::Error,
    },
    /// The steps of `new_project` are not a script Stave can read, or
    /// can't be run.
    #[error("The steps can't be run: {0}")]
    Script(#[from] stave_app::script::ScriptError),
    /// A step of `new_project` failed; nothing was kept. Holds the
    /// transcript up to and including the failure.
    #[error("A step failed, so no project was made. What Stave did:\n{0}")]
    StepFailed(String),
    /// A patch couldn't be tried: the scratch copy didn't load.
    #[error("Stave can't try the patch: {0}")]
    Try(#[from] stave_app::scratch::TryError),
    /// A patch that fit couldn't be built on.
    #[error("Stave can't try the next patch: {0}")]
    Advance(#[from] stave_app::scratch::CommitError),
    /// The patch text is too large to read.
    #[error("Stave can't read the patch: {0}")]
    Sources(#[from] stave_lang::diagnostics::SourceMapError),
    /// `fixes_for` was given a problem number that isn't in the list.
    #[error("There is no problem {number}: the project has {count}. Call check to see them.")]
    NoSuchProblem {
        /// The number asked for, from 1.
        number: usize,
        /// How many problems there are.
        count: usize,
    },
    /// `describe` was given a name that isn't in the design.
    #[error("The design has no part or hardware called '{0}'.")]
    NoSuchItem(String),
    /// The design couldn't be placed, so there is nothing to describe.
    #[error("The design has errors, so it can't be described yet. Call check to see them.")]
    NotPlaced,
    /// `grammar` was given a topic that isn't a section of the language
    /// spec.
    #[error("The language guide has no section called '{topic}'. Its sections are: {sections}")]
    NoSuchTopic {
        /// The topic asked for.
        topic: String,
        /// The section titles, comma-separated.
        sections: String,
    },
    /// `std_lookup` was given a name the standard modules don't declare.
    #[error("The standard modules have no kind or recipe called '{0}'.")]
    NoSuchKind(String),
    /// The work behind a tool stopped unexpectedly.
    #[error("Stave stopped while working on this: {0}")]
    Stopped(#[from] tokio::task::JoinError),
    /// The connection to the AI app couldn't be set up.
    #[error("Stave couldn't start talking to the AI app: {0}")]
    Connection(#[from] Box<rmcp::service::ServerInitializeError>),
    /// The server's runtime couldn't be started.
    #[error("Stave couldn't start the server: {0}")]
    Runtime(std::io::Error),
    /// Where Stave keeps its settings couldn't be worked out.
    #[error("Stave can't find your home folder: HOME is not set.")]
    NoHome,
}
