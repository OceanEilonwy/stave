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
//! ```
//!
//! `check` runs the whole pipeline (through `stave-app`) on a module
//! folder: a folder that holds `.stave` files directly. Given a folder
//! without any, it checks every module folder below it, in sorted order so
//! output is the same on every run. `fmt` searches a directory recursively
//! for `.stave` files. Exit codes: 0 when everything is fine, 1
//! when `check` finds errors or `fmt --check` finds a file to reformat (`fmt`
//! skips files with syntax errors, which are `check`'s business), 2 when
//! the command itself fails (a missing path, an unreadable file).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use stave_lang::{FormatError, Severity, SourceMap, format_file, render_diagnostics};

mod dump;

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

/// Formats every file, or with `check_only` reports the ones that would
/// change. `Ok(true)` when every file that parses is (now) formatted; files
/// with syntax errors are skipped with a note.
fn fmt(path: &Path, check_only: bool) -> Result<bool> {
    let files = stave_files(path)?;
    let mut sources = SourceMap::new();
    let mut all_fine = true;
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    for path in &files {
        let text = read(path)?;
        let file = sources.add(path.display().to_string(), text.as_str())?;
        match format_file(file, &text) {
            Ok(formatted) if formatted == text => {}
            Ok(formatted) => {
                if check_only {
                    writeln!(stdout, "would reformat {}", path.display())?;
                    all_fine = false;
                } else {
                    std::fs::write(path, formatted)
                        .with_context(|| format!("can't write '{}'", path.display()))?;
                    writeln!(stdout, "formatted {}", path.display())?;
                }
            }
            Err(FormatError::Syntax(diagnostics)) => {
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
            Err(error @ FormatError::WouldChangeMeaning) => {
                writeln!(stderr, "not formatted: {}: {error}", path.display())?;
                all_fine = false;
            }
        }
    }
    Ok(all_fine)
}

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 { one } else { many }
}
