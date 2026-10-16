//! The `anicraft` binary.

use std::path::PathBuf;

use anicraft_app::mcp::LinkOptions;
use anicraft_app::{Options, ShotPlan};
use clap::Parser;

/// `AniCraft`: a 2D animation editor for Adobe Animate documents (.fla / XFL).
#[derive(Debug, Parser)]
#[command(version, about)]
#[allow(clippy::struct_excessive_bools, reason = "independent command line switches")]
struct Args {
    /// A document to open (.fla, .xfl folder or its marker file).
    file: Option<PathBuf>,

    /// Load the document, render a frame offscreen, print a JSON report and exit. Opens no window.
    #[arg(long)]
    smoke: bool,

    /// Open the window, show the document, save a PNG of the whole window to this path and exit.
    #[arg(long, value_name = "OUT.png")]
    screenshot: Option<PathBuf>,

    /// The 1-based frame to show (for --screenshot and --smoke).
    #[arg(long, value_name = "N")]
    frame: Option<u32>,

    /// With --screenshot: crop to one panel (stage, timeline, library, properties, tools).
    #[arg(long, value_name = "PANEL")]
    panel: Option<String>,

    /// With --screenshot: activate a tool first (select, rect, oval, pen, ...), to show its overlay.
    #[arg(long, value_name = "TOOL")]
    tool: Option<String>,

    /// With --screenshot: select an element first, as LAYER:KEYFRAME:INDEX (all 0-based).
    #[arg(long, value_name = "L:K:I")]
    select: Option<String>,

    /// With --screenshot: select this library item (its full path) so the Library panel previews it.
    #[arg(long, value_name = "ITEM")]
    library: Option<String>,

    /// With --library: start the preview playing before the shot.
    #[arg(long)]
    library_play: bool,
    /// With --screenshot: open dialogs and panels first (comma separated): new-document, templates, settings, find,
    /// history, output, errors, tabs, save-changes, revert.
    #[arg(long, value_name = "LIST")]
    show: Option<String>,

    /// Window size for --screenshot, as `WIDTHxHEIGHT` in points.
    #[arg(long, value_name = "WxH", default_value = "1440x900")]
    size: String,

    /// Serve MCP on stdin and stdout while the window is open. Nothing else is written to stdout.
    #[arg(long)]
    mcp_stdio: bool,

    /// Listen for the `anim-mcp` bridge on this loopback port (0 picks a free one).
    #[arg(long, value_name = "PORT")]
    control_port: Option<u16>,

    /// Where to write the control token and address (default for a fixed --control-port: `anicraft-control-PORT.token`
    /// in the runtime or temp folder, where `anicraft-cli mcp --bridge PORT` finds it).
    #[arg(long, value_name = "FILE")]
    token_file: Option<PathBuf>,

    /// A folder MCP clients may read and write (repeatable). The folder of the opened file is always allowed.
    #[arg(long = "root", value_name = "DIR")]
    roots: Vec<PathBuf>,

    /// Start in the light theme.
    #[arg(long)]
    light: bool,
}

fn parse_size(text: &str) -> Option<(f32, f32)> {
    let (w, h) = text.split_once(['x', 'X'])?;
    let (w, h) = (w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?);
    (w >= 400.0 && h >= 300.0 && w <= 8000.0 && h <= 8000.0).then_some((w, h))
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let args = Args::parse();
    if args.smoke {
        let report = anicraft_app::smoke::run(args.file.as_deref(), args.frame).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }
    let window = parse_size(&args.size).ok_or_else(|| color_eyre::eyre::eyre!("--size must look like 1440x900"))?;
    let shot = args.screenshot.map(|out| ShotPlan {
        out,
        frame: args.frame,
        panel: args.panel,
        tool: args.tool,
        select: args.select,
        library: args.library,
        library_play: args.library_play,
        show: args.show,
    });
    let options = Options {
        file: args.file,
        shot,
        dark: !args.light,
        links: LinkOptions {
            stdio: args.mcp_stdio,
            control_port: args.control_port,
            token_file: args.token_file.or_else(|| args.control_port.filter(|p| *p != 0).map(anicraft_cli::mcp::default_token_file)),
        },
        roots: args.roots,
        window: Some(window),
    };
    anicraft_app::run(options)
}
