//! `stave-gtk`: the Stave viewer.
//!
//! ```text
//! stave-gtk                                            (the start page)
//! stave-gtk examples/clothing_cabinet
//! stave-gtk examples/clothing_cabinet --screenshot cabinet.png --scene all_open --size 1600x1000
//! stave-gtk examples/wall_shelf --screenshot back.png --turn 180     (seen from behind)
//! stave-gtk examples/side_table --screenshot apart.png --explode 1   (pulled apart)
//! stave-gtk tests/broken/door_hits_side --screenshot hit.png --diagnostic 0
//! stave-gtk examples/clothing_cabinet --window-screenshot main.png --theme dark \
//!     --select 'door_l.stile[hinge]'                    (the real window, see tools/window-shot)
//! stave-gtk examples/clothing_cabinet --screenshot hinge.png --open door_l=45 \
//!     --frame 'door_l.hinges[lower]' --pitch 90 --section up=660   (a hinge from above)
//! ```
//!
//! Without `--screenshot` it opens a libadwaita window for each folder (or
//! the start page, given none), or hands the folders to the Stave already
//! running. With it, it renders one frame offscreen with the default
//! framing, writes a PNG and exits, without touching GTK or a display.

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
    /// The project folders (or a `.stave` file in one); none shows the
    /// start page.
    folders: Vec<PathBuf>,
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
    /// close (for checking the window without looking at a screen; see
    /// `tools/window-shot`).
    #[arg(long, value_name = "PNG")]
    window_screenshot: Option<PathBuf>,
    /// The size in pixels: of the screenshot (default 1600x1000), or of
    /// the window (default: as it was last time, 1200x800 at first;
    /// 1440x900 with --window-screenshot).
    #[arg(long, value_name = "WIDTHxHEIGHT", value_parser = parse_size)]
    size: Option<(u32, u32)>,
    /// The window's style: system, light or dark (default: the one chosen
    /// last time).
    #[arg(long, value_name = "STYLE", value_parser = parse_theme)]
    theme: Option<ui::Theme>,
    /// The view the window starts in: design or cut-list (`parts` and
    /// `files` name the design too; `files` opens the editor beside it).
    #[arg(long, value_name = "VIEW", default_value = "design", value_parser = parse_view)]
    view: (stave_app::session::View, bool),
    /// Open the editor beside the 3D view.
    #[arg(long)]
    editor: bool,
    /// Turn X-ray on in the dock: everything but the selection
    /// see-through.
    #[arg(long)]
    xray: bool,
    /// Open the finder (Ctrl+K) with this typed.
    #[arg(long, value_name = "TEXT")]
    finder: Option<String>,
    /// Lay the panels out as this layout file says (the form Stave keeps
    /// in ~/.config/stave/layout), without reading or writing that file.
    #[arg(long, value_name = "FILE")]
    layout: Option<std::path::PathBuf>,
    /// Open this file in the editor, at a line: `doors.stave:9` (relative
    /// to the folder, or `std/joinery/dado.stave`).
    #[arg(long, value_name = "FILE[:LINE]")]
    file: Option<String>,
    /// In the editor, put the cursor on the first occurrence of this
    /// word on the line `--file` names.
    #[arg(long, value_name = "WORD")]
    cursor: Option<String>,
    /// In the editor, peek at the definition under the cursor.
    #[arg(long)]
    peek: bool,
    /// In the editor, show the card for the word under the cursor.
    #[arg(long)]
    hover: bool,
    /// In the editor, list the uses of the name under the cursor.
    #[arg(long)]
    uses: bool,
    /// In the editor, type this text at the cursor, as if typed.
    #[arg(long = "type", value_name = "TEXT")]
    typing: Option<String>,
    /// In the editor, follow the name under the cursor (F12).
    #[arg(long)]
    follow: bool,
    /// In the editor, undo once (after --type, if given).
    #[arg(long)]
    undo: bool,
    /// In the editor, find this text in the file from the cursor
    /// (Ctrl+F), with every match highlighted.
    #[arg(long, value_name = "TEXT")]
    search: Option<String>,
    /// With --window-screenshot: check again and capture the window while
    /// that check runs (the status pill's spinner).
    #[arg(long)]
    checking: bool,
    /// With --window-screenshot: press the build button and capture the
    /// window while it checks.
    #[arg(long, hide = true)]
    building: bool,
    /// Turn the camera this many degrees about the up axis from the
    /// default view (positive toward the right; 180 shows the back). Works
    /// for the window too.
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
    /// down). Works for the window too.
    #[arg(long, value_name = "DEGREES", allow_negative_numbers = true)]
    pitch: Option<f32>,
    /// Frame the screenshot close on the items whose names start with this
    /// (`door_l.hinges[lower]`: every member of the hinge,
    /// `door_l.hinges[lower] cup` and the rest).
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
    /// Draw the screenshot flat (orthographic), as a drawing would.
    #[arg(long)]
    flat: bool,
    /// Focus on a joint or a piece of hardware by name
    /// (`rung_sockets`, `door_l.hinges[upper]`): everything else fades, the
    /// part receiving a joint is drawn see-through, and the side panel's
    /// text is printed.
    #[arg(long, value_name = "NAME")]
    focus: Option<String>,
    /// Which pair of the focused joint, from 0 (default 0).
    #[arg(long, value_name = "N")]
    pair: Option<usize>,
    /// How far along focus mode is: 0 apart to 1 together for a joint, 0
    /// closed to 1 fully open for hardware (default 0.5; works for the
    /// window too).
    #[arg(long, value_name = "AMOUNT", value_parser = parse_amount)]
    step: Option<f32>,
    /// With --focus on a hinge, a slide or a joint that turns upright: look
    /// straight down, flat, through a section across it (works for the
    /// window too).
    #[arg(long)]
    plan: bool,
    /// With --focus: stay in 3D where focus would open looking from above
    /// (a joint whose part turns upright, the chidori).
    #[arg(long)]
    in_3d: bool,
    /// With --focus: draw the receiving part solid, not see-through.
    #[arg(long)]
    no_xray: bool,
    /// With --focus: keep the other parts as they are, not faded.
    #[arg(long)]
    no_fade: bool,
    /// Don't tint the faces cuts made.
    #[arg(long)]
    no_tint: bool,
    /// For the window: start with the camera framed on the selection (as
    /// pressing F does).
    #[arg(long)]
    frame_selection: bool,
    /// For the window: move the camera this many times closer once framed
    /// (2: half as far away).
    #[arg(long, value_name = "FACTOR", value_parser = parse_zoom)]
    zoom: Option<f32>,
    /// For the window: show the editing menu or card from the contract's
    /// fixture over the design (menu, greyed, refused, fits, choose or point).
    #[arg(long, value_name = "STATE", value_parser = parse_edit_fixture)]
    edit_fixture: Option<ui::Showcase>,
    /// For the window: once the design is drawn, run these editing steps
    /// (hover, select, choose, bind, set, name, fix, add, cancel; see
    /// docs/gui.md), as the pointer and the card would.
    #[arg(long, value_name = "FILE")]
    edit_script: Option<PathBuf>,
    /// For the window: open the New Project dialog.
    #[arg(long)]
    new_project: bool,
}

fn parse_edit_fixture(text: &str) -> Result<ui::Showcase> {
    ui::Showcase::from_word(text.trim()).ok_or_else(|| {
        anyhow::anyhow!("the editing state is menu, greyed, refused, fits, choose or point")
    })
}

fn parse_zoom(text: &str) -> Result<f32> {
    let zoom: f32 = text.trim().parse()?;
    if !(0.05..=100.0).contains(&zoom) {
        bail!("the zoom must be between 0.05 and 100");
    }
    Ok(zoom)
}

fn parse_theme(text: &str) -> Result<ui::Theme> {
    ui::Theme::from_word(text.trim())
        .ok_or_else(|| anyhow::anyhow!("the style is system, light or dark"))
}

/// The view a word names, and whether it opens the editor (`files`).
fn parse_view(text: &str) -> Result<(stave_app::session::View, bool)> {
    let word = text.trim();
    stave_app::session::View::from_word(word)
        .map(|view| (view, word == "files"))
        .ok_or_else(|| anyhow::anyhow!("the view is design or cut-list"))
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
        Some(output) => match arguments.folders.as_slice() {
            [folder] => screenshot::run(
                folder,
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
                    flat: arguments.flat,
                    no_tint: arguments.no_tint,
                    focus: arguments.focus.clone().map(|name| screenshot::FocusShot {
                        name,
                        pair: arguments.pair,
                        step: arguments.step,
                        plan: arguments.plan,
                        in_3d: arguments.in_3d,
                        options: stave_app::focus::Options {
                            xray: !arguments.no_xray,
                            tint_cut_faces: !arguments.no_tint,
                            fade_others: !arguments.no_fade,
                        },
                    }),
                },
                arguments.diagnostic,
                arguments.size.unwrap_or((1600, 1000)),
            ),
            _ => Err(anyhow::anyhow!("a screenshot needs one project folder")),
        },
        None => ui::run(ui::Options {
            folders: arguments.folders,
            scene: arguments.scene,
            select: arguments.select,
            focus: arguments.focus,
            plan: arguments.plan,
            in_3d: arguments.in_3d,
            step: arguments.step,
            problem: arguments.diagnostic,
            explode: arguments.explode,
            view: arguments.view.0,
            editor: arguments.editor || arguments.view.1 || arguments.file.is_some(),
            finder: arguments.finder,
            xray: arguments.xray,
            layout: arguments.layout,
            theme: arguments.theme,
            window_screenshot: arguments.window_screenshot,
            size: arguments.size,
            maximized: false,
            checking: arguments.checking,
            building: arguments.building,
            file: arguments.file,
            wish: ui::Wish {
                cursor: arguments.cursor,
                peek: arguments.peek,
                hover: arguments.hover,
                uses: arguments.uses,
                typing: arguments.typing,
                follow: arguments.follow,
                undo: arguments.undo,
                search: arguments.search,
            },
            camera: ui::StartCamera {
                turn: arguments.turn,
                pitch: arguments.pitch,
                frame_selection: arguments.frame_selection,
                zoom: arguments.zoom,
            },
            edit_fixture: arguments.edit_fixture,
            edit_script: arguments.edit_script,
            new_project: arguments.new_project,
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
