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
    /// The steps of `new_project` ran but never reached `create`, so no
    /// `.stave` file was written; nothing was kept.
    #[error("The steps never reached 'create', so no project was made.")]
    NeverCreated,
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
    /// A tool that works on the project's versions was called while
    /// they are off.
    #[error(
        "Versions are off for this project. Call open_project again with start_versions: true to turn them on."
    )]
    VersionsOff,
    /// The project folder sits inside a bigger repository, so versions
    /// can't be kept for it.
    #[error(
        "'{}' is inside another project's history (at '{}'), so Stave can't keep versions of it there. Copy the project to a folder of its own, outside '{}', and open the copy.",
        .folder.display(), .repository_root.display(), .repository_root.display()
    )]
    Nested {
        /// The project folder.
        folder: PathBuf,
        /// The root of the repository around it.
        repository_root: PathBuf,
    },
    /// The project's versions couldn't do what was asked.
    #[error("{}", sentence(&.0.to_string()))]
    History(#[from] stave_history::HistoryError),
    /// The merge policy is `off`: agents suggest, the person applies.
    #[error("Only the person using Stave can apply suggestions in this project.")]
    ApplyNotAllowed,
    /// The merge policy is `off`, so an agent can't make a new project.
    #[error(
        "Only the person using Stave can make new projects here: agents may only suggest changes."
    )]
    NewProjectNotAllowed,
    /// A patch to suggest doesn't fit, so nothing was recorded.
    #[error("The patch doesn't fit, so nothing was suggested. What Stave found:\n{0}")]
    SuggestionRefused(String),
    /// A patch to suggest fits but leaves the design with errors, so it
    /// could never be applied; nothing was recorded.
    #[error(
        "The patch fits, but the design doesn't check clean afterwards, so it couldn't be applied and nothing was suggested. Fix these errors first, in the same patch or an earlier suggestion:\n{0}"
    )]
    SuggestionNotClean(String),
    /// A suggestion, tried on the project as it is now, leaves errors.
    #[error(
        "The suggestion doesn't check clean on the project as it is now, so it wasn't applied. What Stave found:\n{0}"
    )]
    NotClean(String),
    /// A suggestion's patches no longer fit main, and moving its line
    /// onto main clashed.
    #[error(
        "The suggestion's patches no longer fit the project as it is now, and moving it onto the project clashes in {}. Resolve the clashes in the suggestion, or discard it and suggest the change again.",
        .files.join(", ")
    )]
    Clashes {
        /// The files with clashes.
        files: Vec<String>,
    },
    /// A clash couldn't be settled as asked.
    #[error("Stave can't settle that clash: {0}")]
    Conflict(#[from] stave_app::conflicts::ConflictError),
    /// A file of the project isn't text Stave can read.
    #[error("'{0}' isn't text Stave can read.")]
    NotText(String),
    /// `try_resolution` or `resolve_conflict` named a file with no clash.
    #[error("'{0}' has no clashes. Call list_conflicts to see where they are.")]
    NoConflicts(String),
    /// `try_resolution` or `resolve_conflict` named a clash the file
    /// doesn't have.
    #[error("'{file}' has no clash {number}: it has {count}, numbered from 1.")]
    NoSuchClash {
        /// The file.
        file: String,
        /// The number asked for, from 1.
        number: usize,
        /// How many clashes the file has.
        count: usize,
    },
    /// A resolution kept a side the clash doesn't have.
    #[error("That clash has no side {number}: it has {count}, numbered from 1.")]
    NoSuchSide {
        /// The number asked for, from 1.
        number: usize,
        /// How many sides the clash has.
        count: usize,
    },
    /// A resolution of the project's own files that doesn't check clean
    /// isn't written: an agent's change never reaches the project
    /// unchecked.
    #[error(
        "Nothing was written: with this resolution the project would have {errors}. \
         Settle it another way (try it with try_resolution first), or leave it for the \
         person using Stave.\n{problems}"
    )]
    ResolutionNotClean {
        /// How many errors, in words.
        errors: String,
        /// The problems, as `stave check` prints them.
        problems: String,
    },
    /// The project's merge policy is `off`, so only the person changes its
    /// own files.
    #[error(
        "Only the person using Stave can change this project's own files (its merge policy is \
         off). They can settle the clash in the Stave window."
    )]
    PolicyOff,
    /// The file changed while a clash in it was being settled.
    #[error(
        "'{0}' changed while the clash was being settled, so nothing was written. Call list_conflicts again."
    )]
    ChangedMeanwhile(String),
    /// `preview` or `show_in_stave` named a suggestion that isn't there.
    #[error("There is no suggestion '{0}'. Call list_suggestions to see them.")]
    NoSuchSuggestion(String),
    /// The design places nothing, so there is nothing to draw.
    #[error("The design has nothing to draw yet.")]
    NothingToDraw,
    /// `render_view` was given a camera that can't be.
    #[error("The camera can't be set that way: {0}.")]
    Camera(String),
    /// There is no graphics adapter to draw a picture with.
    #[error(
        "Stave can't draw a picture on this computer: it has no graphics adapter Stave can use \
         ({0}). Call preview for the 3D view in a web browser instead."
    )]
    NoGraphics(String),
    /// The picture couldn't be drawn.
    #[error("Stave couldn't draw the picture: {0}.")]
    Render(stave_render::RenderError),
    /// The localhost viewer couldn't be started.
    #[error("Stave couldn't start the preview: {0}.")]
    Viewer(#[from] crate::viewer_server::ViewerError),
    /// The Stave window isn't running, so it can't show anything.
    #[error("Stave isn't running. Open it and ask again.")]
    StaveNotRunning,
    /// The Stave window couldn't be asked.
    #[error("Stave couldn't ask its window to show the project: {0}.")]
    ShowInStave(#[from] stave_link::LinkError),
}

/// `text` as a sentence: a capital first letter and a full stop.
fn sentence(text: &str) -> String {
    let mut letters = text.chars();
    let mut sentence: String = letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default();
    if !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    sentence
}
