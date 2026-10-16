//! Command-line definition and the commands themselves.

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr as _};

use crate::convert;
use crate::export::{self, ExportJob, Format, RenderSpec};
use crate::info;
use crate::project::{Project, resolve_span};

/// Headless `AniCraft`. Frame numbers are 1-based, like the timeline in Animate.
#[derive(Debug, Parser)]
#[command(name = "anicraft-cli", version, about, long_about = None)]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Describe a document: scenes, layers, frames, library, warnings.
    Info(InfoArgs),
    /// Render one frame to a PNG.
    Render(RenderArgs),
    /// Export frames as PNG sequence, GIF, MP4, MOV, SVG, animated SVG, sprite sheet or SWF.
    Export(ExportArgs),
    /// Convert between `.fla` and XFL folders.
    Convert(ConvertArgs),
    /// Serve the document over the Model Context Protocol on stdin and stdout.
    Mcp(McpArgs),
}

/// Arguments of `info`.
#[derive(Debug, Args)]
pub struct InfoArgs {
    /// A `.fla`, an XFL folder or an `.xfl` marker file.
    pub file: PathBuf,
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
    /// List every layer of every scene.
    #[arg(long)]
    pub layers: bool,
}

/// Arguments of `render`.
#[derive(Debug, Args)]
pub struct RenderArgs {
    /// A `.fla`, an XFL folder or an `.xfl` marker file.
    pub file: PathBuf,
    /// Frame to render, counted from 1.
    #[arg(long, default_value_t = 1)]
    pub frame: u32,
    /// Scene name (default: the scene the document had open).
    #[arg(long)]
    pub scene: Option<String>,
    /// Render this library symbol (cropped to its bounds) instead of a scene; `--frame` then counts in the symbol.
    #[arg(long, conflicts_with = "scene")]
    pub symbol: Option<String>,
    /// Output pixels per stage pixel.
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    /// Leave the stage background out.
    #[arg(long)]
    pub transparent: bool,
    /// Where to write the PNG.
    #[arg(short, long)]
    pub output: PathBuf,
}

/// Arguments of `export`.
#[derive(Debug, Args)]
pub struct ExportArgs {
    /// A `.fla`, an XFL folder or an `.xfl` marker file.
    pub file: PathBuf,
    /// Output format.
    #[arg(long, value_enum)]
    pub format: Format,
    /// Output file, or folder for `png-seq`.
    #[arg(short, long)]
    pub output: PathBuf,
    /// Scene name (default: the scene the document had open).
    #[arg(long)]
    pub scene: Option<String>,
    /// First frame, counted from 1 (default 1).
    #[arg(long)]
    pub from: Option<u32>,
    /// Last frame, inclusive (default: the end of the scene).
    #[arg(long)]
    pub to: Option<u32>,
    /// Output frame rate (default: the document's).
    #[arg(long)]
    pub fps: Option<f64>,
    /// Output pixels per stage pixel.
    #[arg(long, default_value_t = 1.0)]
    pub scale: f64,
    /// Leave the stage background out (needs a format with transparency; for MOV this stores alpha).
    #[arg(long, alias = "alpha")]
    pub transparent: bool,
    /// Do not print progress.
    #[arg(short, long)]
    pub quiet: bool,
}

/// Arguments of `convert`.
#[derive(Debug, Args)]
pub struct ConvertArgs {
    /// Source document.
    pub input: PathBuf,
    /// A `.fla` file, or any other name for an XFL folder (`scene.xfl` is a folder too).
    pub output: PathBuf,
}

/// Arguments of `mcp`.
#[derive(Debug, Args)]
pub struct McpArgs {
    /// A document to open at start (also accepted as a plain argument).
    #[arg(long = "file", value_name = "FILE", conflicts_with = "positional_file")]
    pub file: Option<PathBuf>,
    /// A document to open at start.
    #[arg(value_name = "FILE")]
    pub positional_file: Option<PathBuf>,
    /// Folders the client may read and write (repeatable). Without any root, files are off limits.
    #[arg(long = "root", value_name = "DIR")]
    pub roots: Vec<PathBuf>,
    /// Folders the client may read (repeatable).
    #[arg(long = "read-root", value_name = "DIR")]
    pub read_roots: Vec<PathBuf>,
    /// Folders the client may write (repeatable). Writing does not imply reading.
    #[arg(long = "write-root", value_name = "DIR")]
    pub write_roots: Vec<PathBuf>,
    /// Drive the running editor (`anicraft --control-port PORT`) instead of a headless session, so you can watch it work.
    #[arg(long, value_name = "PORT", conflicts_with_all = ["file", "positional_file", "roots", "read_roots", "write_roots"])]
    pub bridge: Option<u16>,
    /// With --bridge: the token file the editor wrote (default: the one named after the port, else the `ANIM_MCP_TOKEN` variable).
    #[arg(long, value_name = "FILE", requires = "bridge")]
    pub token_file: Option<PathBuf>,
}

/// Runs a parsed command line.
///
/// # Errors
/// Whatever the command reports, with context.
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Info(args) => info_command(&args),
        Command::Render(args) => render_command(&args),
        Command::Export(args) => export_command(&args),
        Command::Convert(args) => convert_command(&args),
        Command::Mcp(args) => crate::mcp::serve(&mcp_options(args)),
    }
}

fn mcp_options(args: McpArgs) -> crate::mcp::ServeOptions {
    let mut read_roots = args.roots.clone();
    read_roots.extend(args.read_roots);
    let mut write_roots = args.roots;
    write_roots.extend(args.write_roots);
    crate::mcp::ServeOptions {
        file: args.file.or(args.positional_file),
        read_roots,
        write_roots,
        bridge: args.bridge.map(|port| crate::mcp::BridgeTarget { port, token_file: args.token_file }),
    }
}

fn open(path: &Path) -> Result<Project> {
    Project::open(path).wrap_err_with(|| format!("could not open {}", path.display()))
}

fn info_command(args: &InfoArgs) -> Result<()> {
    let project = open(&args.file)?;
    let report = info::report(&project);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", info::to_text(&report, args.layers));
    }
    Ok(())
}

fn render_command(args: &RenderArgs) -> Result<()> {
    let project = open(&args.file)?;
    let mut ev = project.evaluator(false);
    let spec = RenderSpec { scale: args.scale, transparent: args.transparent };
    let (image, what) = if let Some(symbol) = &args.symbol {
        let frames = project.document().library.find_symbol(symbol).map_or(1, |s| s.timeline.frame_count());
        let span = resolve_span(frames, Some(args.frame), Some(args.frame))?;
        (export::render_symbol(&mut ev, symbol, span.start, spec)?, format!("symbol {symbol:?} frame {} of {frames}", args.frame))
    } else {
        let scene = project.scene_index(args.scene.as_deref())?;
        let total = ev.scene_frames(scene);
        let span = resolve_span(total, Some(args.frame), Some(args.frame))?;
        let name = project.document().scenes.get(scene).map_or("", |s| s.name.as_str());
        (export::render_frame(&mut ev, scene, span.start, spec)?, format!("scene {name:?} frame {} of {total}", args.frame))
    };
    let png = anim_export::encode_png(image.width, image.height, &anim_render::straight_rgba(&image))?;
    write_file(&args.output, &png)?;
    println!("{}: {what}, {}x{} px", args.output.display(), image.width, image.height);
    Ok(())
}

fn export_command(args: &ExportArgs) -> Result<()> {
    let project = open(&args.file)?;
    let scene = project.scene_index(args.scene.as_deref())?;
    let total = project.evaluator(false).scene_frames(scene);
    let mut span = resolve_span(total, args.from, args.to)?;
    if args.format == Format::Svg {
        // A still: only the first frame of the range.
        span = span.start..span.start.saturating_add(1);
    }
    let job = ExportJob {
        doc: project.document(),
        media: project.media(),
        scene,
        span: span.clone(),
        fps: args.fps,
        scale: args.scale,
        transparent: args.transparent,
        format: args.format,
        out: args.output.clone(),
    };
    let show = !args.quiet && std::io::stderr().is_terminal();
    let mut progress = |done: u32, count: u32| {
        if show {
            eprint!("\r{done}/{count} frames");
            std::io::stderr().flush().ok();
        }
        true
    };
    let outcome = export::run(&job, &mut progress)?;
    if show {
        eprintln!();
    }
    println!("{} {}: {} frame(s) from {}, {} byte(s)", outcome.format.id(), args.output.display(), outcome.frames, export::describe_span(&span), outcome.bytes);
    for note in &outcome.notes {
        eprintln!("note: {note}");
    }
    Ok(())
}

fn convert_command(args: &ConvertArgs) -> Result<()> {
    let report = convert::convert(&args.input, &args.output).wrap_err_with(|| format!("could not convert {}", args.input.display()))?;
    println!(
        "{}: {} entries, {} symbol(s), {} media file(s), {} byte(s)",
        args.output.display(),
        report.entries,
        report.symbols,
        report.media_entries,
        report.bytes
    );
    for w in &report.warnings {
        eprintln!("warning: {w}");
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).wrap_err_with(|| format!("could not create {}", parent.display()))?;
    }
    std::fs::write(path, bytes).wrap_err_with(|| format!("could not write {}", path.display()))
}
