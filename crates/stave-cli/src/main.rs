//! `stave`: check, format and dump `.stave` projects from the command line.
//!
//! ```text
//! stave check examples/clothing_cabinet     # diagnostics; exit 1 on errors
//! stave dump examples/clothing_cabinet      # the solved design as a table
//! stave dump --json examples/clothing_cabinet
//! stave fmt examples/clothing_cabinet       # format in place
//! stave fmt --check examples                # exit 1 if a file would change
//! stave view examples/clothing_cabinet      # the 3D viewer (stave-gtk)
//! stave export examples/clothing_cabinet --gltf cabinet.glb
//! stave export examples/clothing_cabinet --gltf apart.glb --scene drawer_bench
//! stave edit examples/clothing_cabinet --patch shelf.stave   # try a patch
//! stave edit examples/clothing_cabinet --patch shelf.stave --commit
//! stave edit examples/clothing_cabinet_assemblies --region divider.above   # the menu
//! stave edit examples/clothing_cabinet_assemblies --region divider.above \
//!     --choose Shelf --bind "parts side_l side_r" --set headroom=400mm --commit
//! stave edit examples/clothing_cabinet_assemblies --script steps.txt
//! stave edit new_chest --script build.txt   # a script can start a new project
//! stave mcp install                         # set Stave up in the AI apps found
//! stave mcp uninstall --codex
//! ```
//!
//! `check` runs the whole pipeline (through `stave-app`) on a module
//! folder: a folder that holds `.stave` files directly. Given a folder
//! without any, it checks every module folder below it, in sorted order so
//! output is the same on every run. `fmt` searches a directory recursively
//! for `.stave` files. `edit` tries each patch of a patch file on a
//! scratch copy of the project, in order, and stops at the first one that
//! is refused (see [`edit`] for its output). Exit codes: 0 when
//! everything is fine, 1 when `check` finds errors, `edit` refuses a
//! patch or `fmt --check` finds a file to reformat (`fmt`
//! skips files with syntax errors, which are `check`'s business), 2 when
//! the command itself fails (a missing path, an unreadable file).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use stave_app::edit::Verdict;
use stave_app::editing::commit_tried;
use stave_app::format::{FormatTextError, Formatted, format_text};
use stave_app::scratch::{DiskWriter, EditBase, one_patch_each};
use stave_lang::ast;
use stave_lang::{FormatError, Parse, Severity, SourceMap, parse_patch_file, render_diagnostics};
use stave_modules::DiskFiles;

mod dump;
mod mcp_install;
mod script;

#[derive(Debug, Parser)]
#[command(
    name = "stave",
    version,
    about = "Check and format .stave woodworking projects"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Check a project or library: syntax, names, types, placement and
    /// sizes. Prints what is wrong with it.
    Check {
        /// A module folder, a .stave file in one, or a folder to search for
        /// module folders.
        path: PathBuf,
    },
    /// Print the solved design: every part's stock, cut size and position,
    /// then hardware, joints, regions and assemblies.
    Dump {
        /// The project folder.
        path: PathBuf,
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Open the design in the 3D viewer. The viewer reloads when a file
    /// changes.
    View {
        /// The project folder.
        path: PathBuf,
        /// The scene to start in.
        #[arg(long)]
        scene: Option<String>,
    },
    /// Write the design as a binary glTF file, for any glTF viewer or
    /// Blender: everything as built, or posed as a scene (exploded when the
    /// scene says `explode: true`).
    Export {
        /// The project folder.
        path: PathBuf,
        /// The .glb file to write.
        #[arg(long, value_name = "FILE")]
        gltf: PathBuf,
        /// The scene to pose it in (default: rest, everything as built).
        #[arg(long)]
        scene: Option<String>,
        /// How far apart to pull the parts, from 0 (as built) to 1 (fully
        /// exploded); default: the scene's own.
        #[arg(long, value_name = "AMOUNT")]
        explode: Option<f32>,
    },
    /// Edit a project without a window. With --patch, try the patches of
    /// a patch file in order: each is applied to a scratch copy, which is
    /// checked; prints whether each fits or is refused, and why, and stops
    /// at the first refusal. With a selection (--region, --face,
    /// --between), list what can go there, or --choose one, fill it in and
    /// --commit it. With --script, run a file of such steps.
    #[command(group(
        clap::ArgGroup::new("what")
            .required(true)
            .args(["patch", "script", "region", "face", "between"])
    ))]
    Edit {
        /// The project folder.
        path: PathBuf,
        /// The patch file: one or more `patch "..." { ... }` blocks.
        #[arg(long, value_name = "FILE")]
        patch: Option<PathBuf>,
        /// A script of edit steps, one a line: new/import/stock/tools/create
        /// to start a project, select/choose/bind/set/name/repeat to add,
        /// repeat/shares/mirror/hold/remove/rename to change, fix, show,
        /// add, cancel (see docs/editing.md, "Scripts").
        #[arg(long, value_name = "FILE")]
        script: Option<PathBuf>,
        /// Select a space by its region's name: `divider.above`.
        #[arg(long, value_name = "NAME")]
        region: Option<String>,
        /// Select a face of a part: `door_l.stile[hinge].front`.
        #[arg(long, value_name = "PART.FACE")]
        face: Option<String>,
        /// Select the gap between two facing parts.
        #[arg(long, num_args = 2, value_names = ["PART", "PART"])]
        between: Option<Vec<String>>,
        /// Choose a menu entry: `Shelf`, or `casework.Shelf`.
        #[arg(long, value_name = "ENTRY", conflicts_with_all = ["patch", "script"])]
        choose: Option<String>,
        /// Point at what the card asks for: `parts side_l side_r`,
        /// `region <name>`, `face <part>.<face>`, `between <part> <part>`.
        #[arg(long, value_name = "SELECTION", requires = "choose")]
        bind: Vec<String>,
        /// Set a setting: `headroom=400mm`.
        #[arg(long, value_name = "NAME=VALUE", requires = "choose")]
        set: Vec<String>,
        /// Name the new item, in place of the generated name.
        #[arg(long, value_name = "NAME", requires = "choose")]
        name: Option<String>,
        /// Write the files: each patch that fits, or the chosen entry.
        #[arg(long, conflicts_with = "script")]
        commit: bool,
    },
    /// Use Stave from an AI app (Claude Desktop, Claude Code, Codex):
    /// set it up there, or take it out. With no subcommand, run the server
    /// the apps start.
    Mcp {
        #[command(subcommand)]
        command: Option<mcp_install::McpCommand>,
    },
    /// Lay out .stave files canonically, in place. Files with syntax errors
    /// are skipped.
    Fmt {
        /// A .stave file, or a folder to search recursively.
        path: PathBuf,
        /// Change nothing; exit with 1 if a file would be reformatted.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check { path } => check(&path),
        Command::Dump { path, json } => dump(&path, json),
        Command::Fmt { path, check } => fmt(&path, check),
        Command::Mcp {
            command: Some(command),
        } => mcp_install::run(&command),
        Command::Mcp { command: None } => mcp_install::serve(),
        Command::Edit {
            path,
            patch,
            script,
            region,
            face,
            between,
            choose,
            bind,
            set,
            name,
            commit,
        } => match (patch, script) {
            (Some(patch), _) => edit(&path, &patch, commit),
            (None, Some(script)) => edit_script(&path, &script),
            (None, None) => edit_steps(
                &path,
                Steps {
                    region,
                    face,
                    between,
                    choose,
                    bind,
                    set,
                    name,
                    commit,
                },
            ),
        },
        Command::View { path, scene } => view(&path, scene.as_deref()),
        Command::Export {
            path,
            gltf,
            scene,
            explode,
        } => export(&path, &gltf, scene.as_deref(), explode),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

/// The `.stave` files at `path`: the file itself, or every `.stave` file
/// under a folder, sorted. Hidden folders (`.git`) and `target` are skipped.
fn stave_files(path: &Path) -> Result<Vec<PathBuf>> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("can't read '{}'", path.display()))?;
    if metadata.is_file() {
        return Ok(vec![path.to_owned()]);
    }
    let mut files = Vec::new();
    collect(path, &mut files)?;
    if files.is_empty() {
        bail!("no .stave files in '{}'", path.display());
    }
    Ok(files)
}

fn collect(folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = std::fs::read_dir(folder)
        .with_context(|| format!("can't read folder '{}'", folder.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<PathBuf>>>()
        .with_context(|| format!("can't read folder '{}'", folder.display()))?;
    entries.sort();
    for entry in entries {
        let name = entry.file_name().map(|name| name.to_string_lossy());
        if entry.is_dir() {
            let skipped = name.is_some_and(|name| name.starts_with('.') || name == "target");
            if !skipped {
                collect(&entry, files)?;
            }
        } else if entry
            .extension()
            .is_some_and(|extension| extension == "stave")
        {
            files.push(entry);
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("can't read '{}'", path.display()))
}

/// The module folders to check at `path`: the folder of a file, a folder
/// with `.stave` files, or every such folder below it.
fn module_folders(path: &Path) -> Result<Vec<PathBuf>> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("can't read '{}'", path.display()))?;
    if metadata.is_file() {
        let folder = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        return Ok(vec![if folder.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            folder
        }]);
    }
    let mut folders = Vec::new();
    let mut files = Vec::new();
    collect(path, &mut files)?;
    for file in files {
        if let Some(parent) = file.parent()
            && !folders.iter().any(|f: &PathBuf| f == parent)
        {
            folders.push(parent.to_path_buf());
        }
    }
    folders.sort();
    if folders.is_empty() {
        bail!("no .stave files in '{}'", path.display());
    }
    Ok(folders)
}

fn open(folder: &Path) -> Result<stave_app::Opened> {
    stave_app::open(folder).with_context(|| format!("can't open '{}'", folder.display()))
}

/// Runs the pipeline on every module folder and prints the diagnostics.
/// `Ok(true)` when nothing has an error.
fn check(path: &Path) -> Result<bool> {
    let folders = module_folders(path)?;
    let mut errors = 0;
    let mut warnings = 0;
    let mut files = 0;
    let mut stderr = std::io::stderr().lock();
    for folder in &folders {
        let opened = open(folder)?;
        files += opened.root_files;
        let count = |severity: Severity| {
            opened
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == severity)
                .count()
        };
        errors += count(Severity::Error);
        warnings += count(Severity::Warning);
        write!(
            stderr,
            "{}",
            render_diagnostics(&opened.sources, &opened.diagnostics)
        )?;
    }
    writeln!(
        stderr,
        "checked {files} {}: {errors} {}, {warnings} {}",
        plural(files, "file", "files"),
        plural(errors, "error", "errors"),
        plural(warnings, "warning", "warnings"),
    )?;
    Ok(errors == 0)
}

/// Prints the solved design. `Ok(false)` when the design has errors.
fn dump(path: &Path, json: bool) -> Result<bool> {
    let opened = open(path)?;
    let mut stderr = std::io::stderr().lock();
    write!(
        stderr,
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    )?;
    let Some(design) = &opened.design else {
        if opened.error_count() == 0 {
            bail!("'{}' is a library: it has nothing to place", path.display());
        }
        return Ok(false);
    };
    let mut stdout = std::io::stdout().lock();
    if json {
        serde_json::to_writer_pretty(&mut stdout, design)?;
        writeln!(stdout)?;
    } else {
        write!(stdout, "{}", dump::table(design, opened.model.as_ref()))?;
    }
    Ok(true)
}

/// Runs the `stave-gtk` viewer that is installed next to this program.
fn view(path: &Path, scene: Option<&str>) -> Result<bool> {
    let me = std::env::current_exe().context("can't find where stave is installed")?;
    let viewer = me.with_file_name(format!("stave-gtk{}", std::env::consts::EXE_SUFFIX));
    if !viewer.is_file() {
        bail!(
            "the viewer isn't installed: expected '{}' next to stave (build it with `cargo build -p stave-gtk`)",
            viewer.display()
        );
    }
    let mut command = std::process::Command::new(&viewer);
    command.arg(path);
    if let Some(scene) = scene {
        command.arg("--scene").arg(scene);
    }
    let status = command
        .status()
        .with_context(|| format!("can't start '{}'", viewer.display()))?;
    Ok(status.success())
}

/// Writes the design as binary glTF, at rest or posed as a scene.
/// `Ok(false)` when the design has errors.
fn export(path: &Path, output: &Path, scene: Option<&str>, explode: Option<f32>) -> Result<bool> {
    let opened = open(path)?;
    let mut stderr = std::io::stderr().lock();
    write!(
        stderr,
        "{}",
        render_diagnostics(&opened.sources, &opened.diagnostics)
    )?;
    let Some(design) = &opened.design else {
        if opened.error_count() == 0 {
            bail!("'{}' is a library: it has nothing to place", path.display());
        }
        return Ok(false);
    };
    let graph = stave_app::view::scene(design);
    let entry = match scene {
        Some(name) => graph.scene_named(name).with_context(|| {
            let names: Vec<&str> = graph.scenes.iter().map(|s| s.name.as_str()).collect();
            format!("no scene '{name}' (scenes: {})", names.join(", "))
        })?,
        None => graph.scenes.first().context("the design has no scenes")?,
    };
    let amount = explode.unwrap_or(if entry.explode { 1.0 } else { 0.0 });
    let posed = graph.pose_with(entry, &[], amount);
    let glb = graph.to_glb(&posed)?;
    std::fs::write(output, &glb).with_context(|| format!("can't write '{}'", output.display()))?;
    writeln!(
        stderr,
        "wrote {} ({} parts and pieces of hardware)",
        output.display(),
        graph.nodes.len()
    )?;
    Ok(true)
}

/// Tries each patch of the patch file at `patch_path` on the project in
/// `folder`, in order, each on top of the ones before it, and stops at the
/// first refusal. With `commit`, writes the files of each patch that fits
/// before trying the next. Prints to standard output, for each patch tried:
///
/// ```text
/// patch "Add a shelf": fits
///   changes cabinet/carcass.stave        (wrote, with --commit; creates/created for a new file)
/// <each warning the patch causes, as `stave check` prints it>
/// patch "Through the divider": refused
/// <each error the patch causes, as `stave check` prints it>
/// checked 2 of 3 patches: 1 fits, 1 refused
/// ```
///
/// A patch is named by its string; a patch file with syntax errors is
/// tried whole and named by its path. `Ok(false)` on a refusal.
fn edit(folder: &Path, patch_path: &Path, commit: bool) -> Result<bool> {
    let text = read(patch_path)?;
    let name = patch_path.display().to_string();
    let mut sources = SourceMap::new();
    let file = sources.add(name.as_str(), text.as_str())?;
    let patches = one_patch_each(file, &parse_patch_file(file, &text));
    let mut base = EditBase::new(folder, &DiskFiles)
        .with_context(|| format!("can't open '{}'", folder.display()))?;
    let cancel = AtomicBool::new(false);
    let mut stdout = std::io::stdout().lock();
    let (mut fit, mut refused) = (0_usize, 0_usize);
    let mut written = std::collections::BTreeSet::new();
    for patch in &patches {
        let tried = base
            .try_patch(&name, patch, &cancel)
            .with_context(|| format!("can't open '{}'", folder.display()))?;
        let title = title(patch, &name);
        match tried.verdict() {
            Verdict::Fits(_) => {
                writeln!(stdout, "{title}: fits")?;
                fit += 1;
                if commit {
                    // The editor's one writer: it refuses a file that
                    // changed on disk since the project was read.
                    let files = commit_tried(base.opened(), &tried, patch, &mut DiskWriter)?;
                    written.extend(files.into_iter().map(|file| file.path));
                }
                if tried.changed().is_empty() {
                    writeln!(stdout, "  changes nothing")?;
                }
                for file in tried.changed() {
                    let verb = match (commit, file.created) {
                        (false, false) => "changes",
                        (false, true) => "creates",
                        (true, false) => "wrote",
                        (true, true) => "created",
                    };
                    writeln!(stdout, "  {verb} {}", file.path.display())?;
                }
            }
            Verdict::Refused(_) | Verdict::Checking => {
                writeln!(stdout, "{title}: refused")?;
                refused += 1;
            }
        }
        write!(
            stdout,
            "{}",
            render_diagnostics(tried.sources(), &tried.shown())
        )?;
        if refused > 0 {
            break;
        }
        base.advance(tried)?;
    }
    let mut summary = format!(
        "checked {} of {} {}: {fit} {}, {refused} refused",
        fit + refused,
        patches.len(),
        plural(patches.len(), "patch", "patches"),
        plural(fit, "fits", "fit"),
    );
    if commit {
        summary.push_str(&format!(
            ", wrote {} {}",
            written.len(),
            plural(written.len(), "file", "files")
        ));
    }
    writeln!(stdout, "{summary}")?;
    Ok(refused == 0)
}

/// `stave edit --script FILE`: runs the steps of a script file, echoing
/// each before what it prints (see [`script`]). `Ok(false)` when a step
/// fails.
fn edit_script(folder: &Path, script_path: &Path) -> Result<bool> {
    let steps = script::parse_script(&read(script_path)?)
        .with_context(|| format!("in '{}'", script_path.display()))?;
    script::run(folder, &steps, true, &mut std::io::stdout().lock())
}

/// The steps the flags of `stave edit` ask for.
struct Steps {
    region: Option<String>,
    face: Option<String>,
    between: Option<Vec<String>>,
    choose: Option<String>,
    bind: Vec<String>,
    set: Vec<String>,
    name: Option<String>,
    commit: bool,
}

/// `stave edit` with a selection: lists the menu, or chooses an entry,
/// fills it in, shows its card and with `--commit` writes it.
fn edit_steps(folder: &Path, flags: Steps) -> Result<bool> {
    let selection = match (flags.region, flags.face, flags.between) {
        (Some(region), _, _) => script::Selection::Region(region),
        (None, Some(face), _) => script::face(&face)?,
        (None, None, Some(between)) => match between.as_slice() {
            [first, second] => script::Selection::Between(first.clone(), second.clone()),
            _ => bail!("--between takes two parts"),
        },
        (None, None, None) => bail!("name a selection: --region, --face or --between"),
    };
    let mut steps = vec![("select".to_owned(), script::Step::Select(selection))];
    let Some(choose) = flags.choose else {
        steps.push(("list".to_owned(), script::Step::List));
        return script::run(folder, &steps, false, &mut std::io::stdout().lock());
    };
    steps.push(("choose".to_owned(), script::Step::Choose(choose)));
    for bind in &flags.bind {
        let words: Vec<&str> = bind.split_whitespace().collect();
        steps.push((
            "bind".to_owned(),
            script::Step::Bind(script::parse_selection(&words)?),
        ));
    }
    for set in &flags.set {
        steps.push(("set".to_owned(), script::setting(set)?));
    }
    if let Some(name) = flags.name {
        steps.push(("name".to_owned(), script::Step::Name(name)));
    }
    steps.push(("show".to_owned(), script::Step::Show));
    if flags.commit {
        steps.push(("commit".to_owned(), script::Step::Commit));
    }
    script::run(folder, &steps, false, &mut std::io::stdout().lock())
}

/// How `stave edit` names a patch: `patch "Add a shelf"`, or the patch
/// file's path when it isn't one readable patch.
fn title(patch: &Parse<ast::PatchFile>, file: &str) -> String {
    let only = match patch.ast.as_ref().map(|ast| ast.patches.as_slice()) {
        Some([only]) if !patch.has_errors() => only,
        _ => return file.to_owned(),
    };
    // The string as written, quotes included.
    let text = patch.syntax().to_string();
    let span = only.summary.span;
    let written = usize::try_from(span.start())
        .ok()
        .zip(usize::try_from(span.end()).ok())
        .and_then(|(start, end)| text.get(start..end))
        .unwrap_or("\"\"");
    format!("patch {written}")
}

/// Formats every file, or with `check_only` reports the ones that would
/// change. `Ok(true)` when every file that parses is (now) formatted; files
/// with syntax errors are skipped with a note.
fn fmt(path: &Path, check_only: bool) -> Result<bool> {
    let files = stave_files(path)?;
    let mut all_fine = true;
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    for path in &files {
        let text = read(path)?;
        match format_text(&path.display().to_string(), &text) {
            Ok(Formatted::Unchanged) => {}
            Ok(Formatted::Changed(formatted)) => {
                if check_only {
                    writeln!(stdout, "would reformat {}", path.display())?;
                    all_fine = false;
                } else {
                    std::fs::write(path, formatted)
                        .with_context(|| format!("can't write '{}'", path.display()))?;
                    writeln!(stdout, "formatted {}", path.display())?;
                }
            }
            Err(FormatTextError::Format(FormatError::Syntax(diagnostics))) => {
                // Layout is fmt's job; errors are check's. A file that
                // doesn't parse is left alone and doesn't fail the run.
                let errors = diagnostics.iter().filter(|d| d.is_error()).count();
                writeln!(
                    stderr,
                    "skipped {}: {errors} syntax {} (see stave check)",
                    path.display(),
                    plural(errors, "error", "errors")
                )?;
            }
            Err(error @ FormatTextError::Format(FormatError::WouldChangeMeaning)) => {
                writeln!(stderr, "not formatted: {}: {error}", path.display())?;
                all_fine = false;
            }
            Err(FormatTextError::Source(error)) => return Err(error.into()),
        }
    }
    Ok(all_fine)
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}
