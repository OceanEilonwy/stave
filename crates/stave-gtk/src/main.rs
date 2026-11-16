//! `stave-gtk`: the Stave viewer.
//!
//! ```text
//! stave-gtk examples/clothing_cabinet
//! stave-gtk examples/clothing_cabinet --screenshot cabinet.png --scene all_open --size 1600x1000
//! ```
//!
//! Without `--screenshot` it opens a libadwaita window. With it, it renders
//! one frame offscreen with the default framing, writes a PNG and exits,
//! without touching GTK or a display.

#![forbid(unsafe_code)]

mod render;
mod screenshot;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "stave-gtk", version, about = "Show a .stave design in 3D")]
struct Arguments {
    /// The project folder.
    folder: PathBuf,
    /// Render one frame to this PNG and exit.
    #[arg(long, value_name = "PNG")]
    screenshot: Option<PathBuf>,
    /// The scene to show (default: rest, everything as built).
    #[arg(long, value_name = "NAME")]
    scene: Option<String>,
    /// Open with this part or piece of hardware selected
    /// (`door_l.stile[hinge]`).
    #[arg(long, value_name = "NAME")]
    select: Option<String>,
    /// Save the whole window to this PNG once the design is drawn, then
    /// close (for checking the window without looking at a screen).
    #[arg(long, value_name = "PNG", hide = true)]
    window_screenshot: Option<PathBuf>,
    /// The screenshot's size in pixels.
    #[arg(long, value_name = "WIDTHxHEIGHT", default_value = "1600x1000", value_parser = parse_size)]
    size: (u32, u32),
}

fn parse_size(text: &str) -> Result<(u32, u32)> {
    let Some((width, height)) = text.split_once(['x', 'X']) else {
        bail!("write the size as WIDTHxHEIGHT, for example 1600x1000");
    };
    let (width, height): (u32, u32) = (width.trim().parse()?, height.trim().parse()?);
    if !(16..=8192).contains(&width) || !(16..=8192).contains(&height) {
        bail!("the size must be between 16 and 8192 pixels each way");
    }
    Ok((width, height))
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let result = match &arguments.screenshot {
        Some(output) => screenshot::run(
            &arguments.folder,
            output,
            arguments.scene.as_deref(),
            arguments.size,
        ),
        None => ui::run(ui::Options {
            folder: arguments.folder,
            scene: arguments.scene,
            select: arguments.select,
            window_screenshot: arguments.window_screenshot,
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
