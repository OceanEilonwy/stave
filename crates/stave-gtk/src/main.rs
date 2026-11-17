//! `stave-gtk`: the Stave viewer.
//!
//! ```text
//! stave-gtk examples/clothing_cabinet
//! stave-gtk examples/clothing_cabinet --screenshot cabinet.png --scene all_open --size 1600x1000
//! stave-gtk examples/wall_shelf --screenshot back.png --turn 180     (seen from behind)
//! stave-gtk examples/side_table --screenshot apart.png --explode 1   (pulled apart)
//! stave-gtk tests/broken/door_hits_side --screenshot hit.png --diagnostic 0
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
    /// Turn the screenshot's camera this many degrees about the up axis
    /// from the default view (positive toward the right; 180 shows the back).
    #[arg(
        long,
        value_name = "DEGREES",
        default_value_t = 0.0,
        allow_negative_numbers = true
    )]
    turn: f32,
    /// Pull the parts apart along their joints, from 0 (as built) to 1
    /// (fully exploded); default: the scene's own. Works for the window
    /// too.
    #[arg(long, value_name = "AMOUNT", value_parser = parse_amount)]
    explode: Option<f32>,
    /// Pose the design where a problem happens, with its parts tinted red
    /// and the shared wood drawn red: the problem's index, from 0, in the
    /// order `stave check` prints them. Works for the window too.
    #[arg(long, value_name = "N")]
    diagnostic: Option<usize>,
}

fn parse_amount(text: &str) -> Result<f32> {
    let amount: f32 = text.trim().parse()?;
    if !(0.0..=1.0).contains(&amount) {
        bail!("the amount must be between 0 and 1");
    }
    Ok(amount)
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
            &screenshot::Shot {
                scene: arguments.scene.clone(),
                explode: arguments.explode,
                turn: arguments.turn,
            },
            arguments.diagnostic,
            arguments.size,
        ),
        None => ui::run(ui::Options {
            folder: arguments.folder,
            scene: arguments.scene,
            select: arguments.select,
            window_screenshot: arguments.window_screenshot,
            problem: arguments.diagnostic,
            explode: arguments.explode,
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
