//! `stave-gtk`: the Stave viewer.
//!
//! ```text
//! stave-gtk examples/clothing_cabinet
//! stave-gtk examples/clothing_cabinet --screenshot cabinet.png --scene all_open --size 1600x1000
//! stave-gtk examples/wall_shelf --screenshot back.png --turn 180     (seen from behind)
//! stave-gtk examples/side_table --screenshot apart.png --explode 1   (pulled apart)
//! stave-gtk tests/broken/door_hits_side --screenshot hit.png --diagnostic 0
//! stave-gtk examples/clothing_cabinet --screenshot hinge.png --open door_l=45 \
//!     --frame 'door_l.hinges[lower]' --pitch 90 --section up=660   (a hinge from above)
//! ```
//!
//! Without `--screenshot` it opens a libadwaita window. With it, it renders
//! one frame offscreen with the default framing, writes a PNG and exits,
//! without touching GTK or a display.

#![forbid(unsafe_code)]

mod convert;
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
    /// Look from this many degrees above the horizontal (90: straight
    /// down) for the screenshot.
    #[arg(long, value_name = "DEGREES", allow_negative_numbers = true)]
    pitch: Option<f32>,
    /// Frame the screenshot close on the items whose names start with this
    /// (`door_l.hinges[lower]`: the hinge's cup, plate and arm, and links).
    #[arg(long, value_name = "NAME")]
    frame: Option<String>,
    /// Open an assembly for the screenshot: degrees for a door, millimetres
    /// for a drawer (`door_l=45`). May be given more than once.
    #[arg(long, value_name = "ASSEMBLY=AMOUNT", value_parser = parse_open)]
    open: Vec<(String, f32)>,
    /// Hide the items whose names start with this from the screenshot. May
    /// be given more than once.
    #[arg(long, value_name = "NAME")]
    hide: Vec<String>,
    /// Cut the screenshot at a plane and leave out what is above it
    /// (`up=660`), to its right (`across=...`) or in front of it
    /// (`deep=...`), in millimetres. Default: the scene's own section.
    #[arg(long, value_name = "AXIS=MM", value_parser = parse_section)]
    section: Option<(stave_lang::placement::Axis, f32)>,
}

fn parse_open(text: &str) -> Result<(String, f32)> {
    let Some((name, amount)) = text.split_once('=') else {
        bail!("write it as ASSEMBLY=AMOUNT, for example door_l=45");
    };
    Ok((name.trim().to_owned(), amount.trim().parse()?))
}

fn parse_section(text: &str) -> Result<(stave_lang::placement::Axis, f32)> {
    let Some((axis, at)) = text.split_once('=') else {
        bail!("write it as AXIS=MM, for example up=660");
    };
    let Some(axis) = stave_lang::placement::Axis::from_word(axis.trim()) else {
        bail!("the axis is up, across or deep");
    };
    Ok((axis, at.trim().parse()?))
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
                pitch: arguments.pitch,
                frame: arguments.frame.clone(),
                open: arguments.open.clone(),
                hide: arguments.hide.clone(),
                section: arguments.section,
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
