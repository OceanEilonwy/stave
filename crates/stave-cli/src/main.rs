//! `stave`: check and format `.stave` projects from the command line.
//!
//! ```text
//! stave check examples/clothing_cabinet     # diagnostics; exit 1 on errors
//! stave fmt examples/clothing_cabinet       # format in place
//! stave fmt --check examples                # exit 1 if a file would change
//! ```
//!
//! A directory is searched recursively for `.stave` files, in sorted order so
//! output is the same on every run. Exit codes: 0 when everything is fine, 1
//! when a file has errors or (with `fmt --check`) is not formatted, 2 when
//! the command itself fails (a missing path, an unreadable file).

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use stave_lang::{
    Diagnostic, FormatError, Severity, SourceMap, format_file, is_patch_source, parse_file,
    parse_patch_file, render_diagnostics,
};

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
    /// Parse every .stave file and print what is wrong with it.
    Check {
        /// A .stave file, or a folder to search recursively.
        path: PathBuf,
    },
    /// Lay out .stave files canonically, in place.
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
        Command::Fmt { path, check } => fmt(&path, check),
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

/// Parses every file and prints the diagnostics. `Ok(true)` when no file
/// has an error.
fn check(path: &Path) -> Result<bool> {
    let files = stave_files(path)?;
    let mut sources = SourceMap::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for path in &files {
        let text = read(path)?;
        let file = sources.add(path.display().to_string(), text.as_str())?;
        if is_patch_source(&text) {
            diagnostics.extend(parse_patch_file(file, &text).diagnostics);
        } else {
            diagnostics.extend(parse_file(file, &text).diagnostics);
        }
    }
    let count = |severity: Severity| {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == severity)
            .count()
    };
    let errors = count(Severity::Error);
    let warnings = count(Severity::Warning);
    let mut stderr = std::io::stderr().lock();
    write!(stderr, "{}", render_diagnostics(&sources, &diagnostics))?;
    writeln!(
        stderr,
        "checked {} {}: {errors} {}, {warnings} {}",
        files.len(),
        plural(files.len(), "file", "files"),
        plural(errors, "error", "errors"),
        plural(warnings, "warning", "warnings"),
    )?;
    Ok(errors == 0)
}

/// Formats every file, or with `check_only` reports the ones that would
/// change. `Ok(true)` when every file is (now) formatted.
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
                write!(stderr, "{}", render_diagnostics(&sources, &diagnostics))?;
                writeln!(stderr, "not formatted: {} has errors", path.display())?;
                all_fine = false;
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
