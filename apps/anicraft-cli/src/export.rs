//! Rendering and exporting a scene.
//!
//! The one place that wires `anicraft-xfl` documents through `anim-timeline` into `anim-render`, `anim-export`,
//! `anim-video` and `anicraft-swf`. The command line and the MCP server both call it.

use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

use anim_core::{Affine, FrameRate, Size, num};
use anim_display::Image;
use anim_export::{AnimatedSvgOptions, ExportBackground, GifOptions, PngSequenceOptions, Progress, RasterOptions, SheetOptions, SvgOptions, VecFrameSource};
use anim_render::{Background, RenderOptions, Renderer};
use anim_scene::{Document, MediaStore};
use anim_timeline::Evaluator;
use anim_video::{AudioTrack, Codec, H264Rate, VideoSettings};
use serde::Serialize;

use crate::audio;
use crate::error::{CliError, Result};
use crate::project::{evaluator_for, frame_plan, one_based};

/// Called with `(done, total)` before each unit of work; returning false cancels the export.
pub type ProgressCb<'a> = &'a mut dyn FnMut(u32, u32) -> bool;

/// An export format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// One PNG per frame into a folder.
    #[value(name = "png-seq")]
    PngSeq,
    /// Animated GIF.
    Gif,
    /// H.264 video with AAC audio from the document's sound layers.
    Mp4,
    /// `ProRes` 4444 video (optionally with alpha) with PCM audio.
    Mov,
    /// A still SVG of one frame.
    Svg,
    /// An animated SVG (SMIL).
    #[value(name = "svg-anim")]
    SvgAnim,
    /// A packed PNG sheet with a JSON description.
    Spritesheet,
    /// A Flash movie.
    Swf,
}

impl Format {
    /// Every format, in the order the help lists them.
    pub const ALL: [Self; 8] = [Self::PngSeq, Self::Gif, Self::Mp4, Self::Mov, Self::Svg, Self::SvgAnim, Self::Spritesheet, Self::Swf];

    /// The name used on the command line and in MCP.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::PngSeq => "png-seq",
            Self::Gif => "gif",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Svg => "svg",
            Self::SvgAnim => "svg-anim",
            Self::Spritesheet => "spritesheet",
            Self::Swf => "swf",
        }
    }

    /// The file extension of the output, empty when the output is a folder.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::PngSeq => "",
            Self::Gif => "gif",
            Self::Mp4 => "mp4",
            Self::Mov => "mov",
            Self::Svg | Self::SvgAnim => "svg",
            Self::Spritesheet => "png",
            Self::Swf => "swf",
        }
    }

    /// A one-line description.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::PngSeq => "one PNG per frame into a folder",
            Self::Gif => "animated GIF (1-bit transparency)",
            Self::Mp4 => "H.264 video with the document's sound layers as AAC",
            Self::Mov => "ProRes 4444 video, with alpha when requested",
            Self::Svg => "still SVG of the first frame of the range",
            Self::SvgAnim => "animated SVG (SMIL)",
            Self::Spritesheet => "packed PNG sprite sheet plus a JSON file next to it",
            Self::Swf => "Flash movie",
        }
    }

    /// Parses the name used on the command line.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == name)
    }
}

/// What to export.
#[derive(Debug, Clone)]
pub struct ExportJob<'a> {
    /// The document.
    pub doc: &'a Document,
    /// Its bitmaps and sounds.
    pub media: &'a MediaStore,
    /// Scene index.
    pub scene: usize,
    /// 0-based half-open frame range.
    pub span: Range<u32>,
    /// Output frame rate, the document's own when `None`.
    pub fps: Option<f64>,
    /// Output pixels per stage pixel.
    pub scale: f64,
    /// Render without the stage background (PNG, GIF, SVG, sprite sheets, `ProRes` with alpha).
    pub transparent: bool,
    /// The format.
    pub format: Format,
    /// File, or folder for `png-seq`.
    pub out: PathBuf,
}

/// What an export wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportOutcome {
    /// The format that was written.
    pub format: Format,
    /// Output frames.
    pub frames: u32,
    /// Files written (one folder entry for `png-seq`).
    pub files: Vec<PathBuf>,
    /// Total bytes written.
    pub bytes: u64,
    /// Things that were approximated or left out.
    pub notes: Vec<String>,
}

/// Render settings for single frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderSpec {
    /// Output pixels per stage pixel.
    pub scale: f64,
    /// Leave the stage background out.
    pub transparent: bool,
}

impl Default for RenderSpec {
    fn default() -> Self {
        Self { scale: 1.0, transparent: false }
    }
}

fn checked_scale(scale: f64) -> Result<()> {
    if scale.is_finite() && scale > 0.0 && scale <= 64.0 { Ok(()) } else { Err(CliError::usage(format!("--scale {scale} must be between 0 and 64"))) }
}

fn render_options(size: Size, spec: RenderSpec, even: bool) -> RenderOptions {
    let base = RenderOptions::scaled_stage(size, spec.scale);
    let options = if even {
        // H.264 needs even dimensions: fit the stage into the next smaller even size (a loss of at most one pixel).
        let down = |n: u32| (n & !1).max(2);
        RenderOptions::fit_stage(size, down(base.width), down(base.height))
    } else {
        base
    };
    if spec.transparent { options.with_background(Background::Transparent) } else { options }
}

/// Renders one frame (0-based) of a scene to premultiplied pixels.
///
/// # Errors
/// [`CliError::Usage`] for a bad scale, [`CliError::Render`] when rasterising fails.
pub fn render_frame(ev: &mut Evaluator<'_>, scene: usize, frame: u32, spec: RenderSpec) -> Result<Image> {
    checked_scale(spec.scale)?;
    let list = ev.frame(scene, frame);
    let options = render_options(list.size, spec, false);
    Ok(anim_render::render(&list, ev.images(), &options)?)
}

/// Renders one frame (0-based) of a library symbol, cropped to the symbol's own bounds plus a margin.
///
/// # Errors
/// [`CliError::Usage`] for an unknown symbol or a bad scale, [`CliError::Render`] when rasterising fails.
pub fn render_symbol(ev: &mut Evaluator<'_>, name: &str, frame: u32, spec: RenderSpec) -> Result<Image> {
    const MARGIN: f64 = 8.0;
    checked_scale(spec.scale)?;
    let bounds =
        ev.symbol_bounds(name, frame).ok_or_else(|| CliError::usage(format!("no symbol named {name:?}, or it is empty at frame {}", one_based(frame))))?;
    let list = ev.frame_of_symbol(name, frame);
    let pixels = |extent: f64| u32::try_from(num::ceil_f64_to_i32((2.0_f64.mul_add(MARGIN, extent) * spec.scale).max(1.0))).unwrap_or(1).max(1);
    let view = Affine::scale(spec.scale) * Affine::translate((MARGIN - bounds.x0, MARGIN - bounds.y0));
    let background = if spec.transparent { Background::Transparent } else { Background::Solid(anim_core::Rgba8::WHITE) };
    let options = RenderOptions::new(pixels(bounds.width()), pixels(bounds.height())).with_view(view).with_background(background);
    Ok(anim_render::render(&list, ev.images(), &options)?)
}

/// Renders one frame (0-based) of a scene to PNG bytes.
///
/// # Errors
/// As [`render_frame`], plus [`CliError::Export`] when encoding fails.
pub fn render_png(doc: &Document, media: &MediaStore, scene: usize, frame: u32, spec: RenderSpec) -> Result<Vec<u8>> {
    let mut ev = evaluator_for(doc, media, false);
    let image = render_frame(&mut ev, scene, frame, spec)?;
    Ok(anim_export::encode_png(image.width, image.height, &anim_render::straight_rgba(&image))?)
}

/// Runs an export.
///
/// # Errors
/// Any of the engine errors, [`CliError::Usage`] for settings that do not fit the format, and
/// [`anim_export::ExportError::Cancelled`] when `progress` returns false.
pub fn run(job: &ExportJob<'_>, progress: ProgressCb<'_>) -> Result<ExportOutcome> {
    checked_scale(job.scale)?;
    let plan = frame_plan(&job.span, job.doc.frame_rate, job.fps)?;
    let out_fps = job.fps.unwrap_or(job.doc.frame_rate);
    let spec = RenderSpec { scale: job.scale, transparent: job.transparent };
    match job.format {
        Format::PngSeq => png_sequence(job, &plan, spec, progress),
        Format::Mp4 | Format::Mov => video(job, &plan, out_fps, spec, progress),
        Format::Gif | Format::Svg | Format::SvgAnim | Format::Spritesheet | Format::Swf => {
            let (source, notes) = collect(job, &plan, out_fps)?;
            from_source(job, &source, out_fps, spec, notes, progress)
        }
    }
}

fn png_sequence(job: &ExportJob<'_>, plan: &[u32], spec: RenderSpec, progress: ProgressCb<'_>) -> Result<ExportOutcome> {
    fs::create_dir_all(&job.out).map_err(|e| CliError::io(&job.out, e))?;
    let names = PngSequenceOptions::default();
    let total = u32::try_from(plan.len()).unwrap_or(u32::MAX);
    let mut ev = evaluator_for(job.doc, job.media, true);
    let mut renderer = Renderer::new();
    let mut bytes = 0_u64;
    for (ordinal, frame) in (0_u32..).zip(plan) {
        if !progress(ordinal, total) {
            return Err(anim_export::ExportError::Cancelled.into());
        }
        let list = ev.frame(job.scene, *frame);
        let image = renderer.render(&list, ev.images(), &render_options(list.size, spec, false))?;
        let png = anim_export::encode_png(image.width, image.height, &anim_render::straight_rgba(&image))?;
        let path = job.out.join(names.file_name(ordinal, total));
        bytes = bytes.saturating_add(u64::try_from(png.len()).unwrap_or(u64::MAX));
        fs::write(&path, png).map_err(|e| CliError::io(path, e))?;
        renderer.recycle(image);
    }
    progress(total, total);
    Ok(ExportOutcome { format: job.format, frames: total, files: vec![job.out.clone()], bytes, notes: Vec::new() })
}

fn video(job: &ExportJob<'_>, plan: &[u32], out_fps: f64, spec: RenderSpec, progress: ProgressCb<'_>) -> Result<ExportOutcome> {
    let first = plan.first().copied().ok_or_else(|| CliError::usage("nothing to export"))?;
    let mut ev = evaluator_for(job.doc, job.media, true);
    let size = ev.frame(job.scene, first).size;
    let mp4 = job.format == Format::Mp4;
    let options = render_options(size, spec, mp4);
    let codec = if mp4 { Codec::H264 { rate: H264Rate::default() } } else { Codec::ProRes4444 { alpha: spec.transparent } };
    let mut settings = VideoSettings::new(options.width, options.height, FrameRate::from_f64(out_fps), codec);
    let mut notes = Vec::new();
    let soundtrack = audio::soundtrack(job.doc, job.media, job.scene, &job.span)?;
    notes.extend(soundtrack.notes);
    if let Some(track) = soundtrack.track {
        settings = settings.with_audio(AudioTrack { samples: track.samples, sample_rate: track.sample_rate, channels: track.channels });
    }
    if let Some(parent) = job.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| CliError::io(parent, e))?;
    }
    let mut handle = anim_video::start(&job.out, settings)?;
    let mut renderer = Renderer::new();
    let total = u32::try_from(plan.len()).unwrap_or(u32::MAX);
    for (ordinal, frame) in (0_u32..).zip(plan) {
        if !progress(ordinal, total) {
            handle.cancel();
            return Err(anim_export::ExportError::Cancelled.into());
        }
        let list = ev.frame(job.scene, *frame);
        handle.send_frame(renderer.render(&list, ev.images(), &options)?)?;
    }
    let summary = handle.finish()?;
    progress(total, total);
    Ok(ExportOutcome { format: job.format, frames: total, files: vec![job.out.clone()], bytes: summary.bytes, notes })
}

/// Evaluates the planned frames up front (formats that need all of them at once).
fn collect(job: &ExportJob<'_>, plan: &[u32], out_fps: f64) -> Result<(VecFrameSource, Vec<String>)> {
    let mut ev = evaluator_for(job.doc, job.media, true);
    let frames: Vec<_> = plan.iter().map(|f| ev.frame(job.scene, *f)).collect();
    let size = frames.first().map(|l| l.size).ok_or_else(|| CliError::usage("nothing to export"))?;
    Ok((VecFrameSource::new(size, out_fps, frames, ev.images().clone()), Vec::new()))
}

fn raster(spec: RenderSpec) -> RasterOptions {
    let background = if spec.transparent { ExportBackground::Transparent } else { ExportBackground::Stage };
    RasterOptions::new().with_scale(spec.scale).with_background(background)
}

fn from_source(
    job: &ExportJob<'_>,
    source: &VecFrameSource,
    out_fps: f64,
    spec: RenderSpec,
    mut notes: Vec<String>,
    progress: ProgressCb<'_>,
) -> Result<ExportOutcome> {
    use anim_export::FrameSource as _;
    if let Some(parent) = job.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| CliError::io(parent, e))?;
    }
    let mut adapter = |p: Progress| progress(p.done, p.total);
    let frames = source.frame_count();
    let out = &job.out;
    let (files, bytes) = match job.format {
        Format::Gif => {
            let report = anim_export::export_gif_file(source, out, &GifOptions { raster: raster(spec), ..GifOptions::default() }, &mut adapter)?;
            (vec![out.clone()], report.bytes)
        }
        Format::Svg => {
            let svg_options = SvgOptions { scale: spec.scale, background: !spec.transparent, ..SvgOptions::default() };
            let text = anim_export::frame_to_svg(source, 0, &svg_options)?;
            write_text(out, &text)?;
            (vec![out.clone()], u64::try_from(text.len()).unwrap_or(u64::MAX))
        }
        Format::SvgAnim => {
            let svg = SvgOptions { scale: spec.scale, background: !spec.transparent, ..SvgOptions::default() };
            let text = anim_export::export_animated_svg(source, &AnimatedSvgOptions { svg, ..AnimatedSvgOptions::default() }, &mut adapter)?;
            write_text(out, &text)?;
            (vec![out.clone()], u64::try_from(text.len()).unwrap_or(u64::MAX))
        }
        Format::Spritesheet => {
            let json: PathBuf = out.with_extension("json");
            let report = anim_export::export_sprite_sheet(source, out, &json, &SheetOptions { raster: raster(spec), ..SheetOptions::default() }, &mut adapter)?;
            (vec![out.clone(), json], report.bytes)
        }
        Format::Swf => {
            let first = source.display_list(0);
            let output = anicraft_swf::export_display_lists(
                (0..frames).map(|i| source.display_list(i)).collect::<Vec<_>>().iter(),
                source.images(),
                anicraft_swf::ExportOptions::for_list(&first, out_fps),
            )?;
            notes.extend(output.report.notes.iter().map(|n| format!("swf: {n:?}")));
            fs::write(out, &output.swf).map_err(|e| CliError::io(out, e))?;
            (vec![out.clone()], u64::try_from(output.swf.len()).unwrap_or(u64::MAX))
        }
        Format::PngSeq | Format::Mp4 | Format::Mov => return Err(CliError::usage("internal: streaming format routed to the collector")),
    };
    Ok(ExportOutcome { format: job.format, frames, files, bytes, notes })
}

fn write_text(path: &Path, text: &str) -> Result<()> {
    fs::write(path, text).map_err(|e| CliError::io(path, e))
}

/// A one-line description of what a job covers, for progress messages.
#[must_use]
pub fn describe_span(span: &Range<u32>) -> String {
    format!("frames {}-{}", one_based(span.start), span.end)
}
