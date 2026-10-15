//! `info`: a description of a document as plain data, printed as text or JSON, and reused by the MCP outline.

use std::fmt::Write as _;

use anicraft_xfl::LoadSummary;
use anim_core::Rgba8;
use anim_scene::{Document, LibraryItem, Timeline};
use serde::Serialize;

use crate::project::Project;

/// Most symbols listed individually; the rest are counted.
pub const MAX_LISTED_SYMBOLS: usize = 200;

/// One layer of a timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayerInfo {
    /// Index in the timeline, top layer first.
    pub index: usize,
    /// The layer's name.
    pub name: String,
    /// Normal, Guide, Guided, Mask, Masked, Folder or Camera.
    pub kind: String,
    /// The folder, mask or guide layer this one belongs to.
    pub parent: Option<usize>,
    /// Shown on stage.
    pub visible: bool,
    /// Locked against editing.
    pub locked: bool,
    /// Number of keyframes.
    pub keyframes: usize,
    /// Length in frames.
    pub frames: u32,
    /// Keyframes that carry a sound.
    pub sounds: usize,
}

/// One scene or symbol timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimelineInfo {
    /// Name of the scene or the library path of the symbol.
    pub name: String,
    /// Length in frames.
    pub frames: u32,
    /// Layers, top first.
    pub layers: Vec<LayerInfo>,
    /// Frame labels as `(1-based frame, name)`.
    pub labels: Vec<(u32, String)>,
}

/// One library symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SymbolInfo {
    /// Library path.
    pub name: String,
    /// Graphic, `MovieClip` or Button.
    pub kind: String,
    /// Length in frames.
    pub frames: u32,
    /// Number of layers.
    pub layers: usize,
}

/// Library contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryInfo {
    /// Symbols, up to [`MAX_LISTED_SYMBOLS`].
    pub symbols: Vec<SymbolInfo>,
    /// Symbols beyond the listed ones.
    pub symbols_not_listed: usize,
    /// Names of bitmaps.
    pub bitmaps: Vec<String>,
    /// Names of sounds.
    pub sounds: Vec<String>,
    /// Names of fonts.
    pub fonts: Vec<String>,
    /// Names of videos.
    pub videos: Vec<String>,
    /// Folders.
    pub folders: usize,
}

/// Stage settings.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StageInfo {
    /// Width in pixels.
    pub width: f64,
    /// Height in pixels.
    pub height: f64,
    /// Frames per second.
    pub fps: f64,
    /// `#rrggbb`.
    pub background: String,
}

/// Everything `info` reports.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InfoReport {
    /// The file that was read.
    pub file: String,
    /// Stage size, rate and colour.
    pub stage: StageInfo,
    /// Scenes in playback order.
    pub scenes: Vec<TimelineInfo>,
    /// Library contents.
    pub library: LibraryInfo,
    /// Counts from the reader.
    pub summary: LoadSummary,
    /// Problems the reader met.
    pub warnings: Vec<String>,
    /// Inconsistencies `anim-scene` found in the model.
    pub validation: Vec<String>,
}

fn hex(c: Rgba8) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// Describes a timeline.
#[must_use]
pub fn timeline_info(timeline: &Timeline) -> TimelineInfo {
    let layers = timeline
        .layers
        .iter()
        .enumerate()
        .map(|(index, layer)| LayerInfo {
            index,
            name: layer.name.clone(),
            kind: format!("{:?}", layer.kind),
            parent: layer.parent_layer_index,
            visible: layer.visible,
            locked: layer.locked,
            keyframes: layer.frames.len(),
            frames: layer.frame_count(),
            sounds: layer.frames.iter().filter(|f| f.sound.is_some()).count(),
        })
        .collect();
    let labels = timeline.labels().into_iter().map(|(frame, name)| (frame.saturating_add(1), name.to_owned())).collect();
    TimelineInfo { name: timeline.name.clone(), frames: timeline.frame_count(), layers, labels }
}

/// Describes the library of a document.
#[must_use]
pub fn library_info(doc: &Document) -> LibraryInfo {
    let mut info = LibraryInfo { symbols: Vec::new(), symbols_not_listed: 0, bitmaps: Vec::new(), sounds: Vec::new(), fonts: Vec::new(), videos: Vec::new(), folders: 0 };
    for item in &doc.library.items {
        match item {
            LibraryItem::Symbol(s) if info.symbols.len() < MAX_LISTED_SYMBOLS => {
                info.symbols.push(SymbolInfo { name: s.name.clone(), kind: format!("{:?}", s.kind), frames: s.timeline.frame_count(), layers: s.timeline.layers.len() });
            }
            LibraryItem::Symbol(_) => info.symbols_not_listed = info.symbols_not_listed.saturating_add(1),
            LibraryItem::Bitmap(b) => info.bitmaps.push(b.name.clone()),
            LibraryItem::Sound(s) => info.sounds.push(s.name.clone()),
            LibraryItem::Font(f) => info.fonts.push(f.name.clone()),
            LibraryItem::Video(v) => info.videos.push(v.name.clone()),
            LibraryItem::Folder(_) => info.folders = info.folders.saturating_add(1),
        }
    }
    info
}

/// Builds the report for an opened project.
#[must_use]
pub fn report(project: &Project) -> InfoReport {
    let doc = project.document();
    InfoReport {
        file: project.path.display().to_string(),
        stage: StageInfo { width: doc.width, height: doc.height, fps: doc.frame_rate, background: hex(doc.background_color) },
        scenes: doc.scenes.iter().map(timeline_info).collect(),
        library: library_info(doc),
        summary: project.loaded.summary(8),
        warnings: project.loaded.warnings.iter().map(ToString::to_string).collect(),
        validation: doc.validate().iter().map(ToString::to_string).collect(),
    }
}

/// The report as text for people. `layers` lists every layer of every scene; otherwise only counts.
#[must_use]
pub fn to_text(report: &InfoReport, layers: bool) -> String {
    let mut out = String::new();
    let s = &report.stage;
    let sum = &report.summary;
    writeln!(out, "{}", report.file).ok();
    writeln!(out, "stage {}x{} px at {} fps, background {}", s.width, s.height, s.fps, s.background).ok();
    writeln!(out, "{} scene(s), {} frame(s) in total, {} layer(s), {} keyframe(s), {} element(s)", sum.scenes, sum.total_frames, sum.layers, sum.keyframes, sum.elements).ok();
    for (i, scene) in report.scenes.iter().enumerate() {
        writeln!(out, "  scene {} {:?}: {} frame(s), {} layer(s)", i.saturating_add(1), scene.name, scene.frames, scene.layers.len()).ok();
        if layers {
            for l in &scene.layers {
                let flags = format!("{}{}", if l.visible { "" } else { " hidden" }, if l.locked { " locked" } else { "" });
                writeln!(out, "    [{}] {:?} {} - {} keyframe(s), {} frame(s){flags}", l.index, l.name, l.kind, l.keyframes, l.frames).ok();
            }
        }
    }
    let lib = &report.library;
    let listed = lib.symbols.len().saturating_add(lib.symbols_not_listed);
    writeln!(out, "library: {listed} symbol(s), {} bitmap(s), {} sound(s), {} font(s), {} video(s), {} folder(s)", lib.bitmaps.len(), lib.sounds.len(), lib.fonts.len(), lib.videos.len(), lib.folders).ok();
    for name in lib.bitmaps.iter().chain(&lib.sounds).chain(&lib.videos) {
        writeln!(out, "  media {name:?}").ok();
    }
    writeln!(out, "media: {} blob(s), {} byte(s)", sum.media_blobs, sum.media_bytes).ok();
    writeln!(out, "kept as unknown: {} attribute(s)/element(s)", sum.unknown_total).ok();
    for (name, count) in sum.top_unknowns.iter().take(5) {
        writeln!(out, "  {name} x{count}").ok();
    }
    writeln!(out, "warnings: {} (+{} dropped)", report.warnings.len(), sum.dropped_warnings).ok();
    for w in report.warnings.iter().take(10) {
        writeln!(out, "  {w}").ok();
    }
    writeln!(out, "validation: {} note(s)", report.validation.len()).ok();
    for w in report.validation.iter().take(10) {
        writeln!(out, "  {w}").ok();
    }
    out
}
