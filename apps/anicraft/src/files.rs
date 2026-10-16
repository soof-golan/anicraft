//! Files: opening, saving, importing artwork and exporting. Everything that touches the disk lives here so the window
//! code stays about state.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anicraft_cli::export::{self, ExportJob, ExportOutcome, Format};
use anicraft_cli::mcp::AniDoc;
use anicraft_cli::project::{Project, evaluator_for, resolve_span};
use anicraft_xfl::WriteOptions;
use anim_core::{Affine, num};
use anim_scene::{BitmapCompression, BitmapInstance, BitmapItem, Element, LibraryItem, TimelineId};
use anim_session::{JobContext, JobError};

/// Result of a background job.
#[derive(Debug)]
pub enum Loaded {
    /// A document was read.
    Opened { path: PathBuf, project: Box<Project> },
    /// An export finished.
    Exported { label: String, outcome: ExportOutcome },
}

/// File extensions the Open dialog offers.
pub const OPEN_EXTENSIONS: [&str; 2] = ["fla", "xfl"];

/// Extensions of artwork the importer understands.
pub const IMPORT_EXTENSIONS: [&str; 11] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "psd", "psb", "ai", "svg", "pdf"];

/// A job body that reads a document.
pub fn open_job(path: PathBuf) -> impl FnOnce(&mut JobContext<Loaded>) -> Result<Loaded, JobError> + Send + 'static {
    move |ctx| {
        ctx.progress(0.05, format!("reading {}", path.display()));
        let project = Project::open(&path).map_err(|e| JobError::Failed(format!("{}: {e}", path.display())))?;
        Ok(Loaded::Opened { path, project: Box::new(project) })
    }
}

/// What an export needs, cloned from the document so the job owns it.
#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub doc: anim_scene::Document,
    pub media: anim_scene::MediaStore,
    pub scene: usize,
    pub frame: u32,
    pub format: Format,
    pub out: PathBuf,
    pub scale: f64,
}

/// A job body that exports.
pub fn export_job(request: ExportRequest) -> impl FnOnce(&mut JobContext<Loaded>) -> Result<Loaded, JobError> + Send + 'static {
    move |ctx| {
        let total = evaluator_for(&request.doc, &request.media, false).scene_frames(request.scene);
        let mut span = resolve_span(total, None, None).map_err(|e| JobError::Failed(e.to_string()))?;
        if request.format == Format::Svg {
            span = request.frame.min(total.saturating_sub(1))..request.frame.min(total.saturating_sub(1)).saturating_add(1);
        }
        let job = ExportJob {
            doc: &request.doc,
            media: &request.media,
            scene: request.scene,
            span,
            fps: None,
            scale: request.scale,
            transparent: request.format == Format::Mov,
            format: request.format,
            out: request.out.clone(),
        };
        let mut progress = |done: u32, count: u32| {
            let fraction = done.saturating_mul(1000).checked_div(count).map_or(1.0, |m| f32::from(u16::try_from(m).unwrap_or(1000)) / 1000.0);
            ctx.progress(fraction, format!("{done}/{count} frames"));
            !ctx.is_cancelled()
        };
        let outcome = export::run(&job, &mut progress).map_err(|e| match e {
            anicraft_cli::CliError::Export(anim_export::ExportError::Cancelled) => JobError::Cancelled,
            other => JobError::Failed(other.to_string()),
        })?;
        Ok(Loaded::Exported { label: request.format.id().to_owned(), outcome })
    }
}

/// Writes the document as a zipped FLA.
///
/// # Errors
/// Writer and I/O errors.
pub fn write_fla(doc: &AniDoc, file: &mut File) -> io::Result<()> {
    let (bytes, _report) = anicraft_xfl::write_fla_bytes(doc.document(), &doc.media, &doc.sidecars, &WriteOptions::default()).map_err(io::Error::other)?;
    file.write_all(&bytes)
}

/// Writes the document as an XFL folder.
///
/// # Errors
/// Writer and I/O errors.
pub fn write_xfl(doc: &AniDoc, folder: &Path) -> Result<(), String> {
    anicraft_xfl::write_xfl_folder(folder, doc.document(), &doc.media, &doc.sidecars, &WriteOptions::default()).map(|_| ()).map_err(|e| e.to_string())
}

/// The current frame as PNG bytes.
///
/// # Errors
/// Render and encode errors.
pub fn current_frame_png(doc: &AniDoc, scale: f64) -> Result<Vec<u8>, String> {
    let scene = match &doc.editor.state.timeline {
        TimelineId::Scene(i) => (*i).min(doc.document().scenes.len().saturating_sub(1)),
        TimelineId::Symbol(_) => doc.document().current_timeline.min(doc.document().scenes.len().saturating_sub(1)),
    };
    export::render_png(doc.document(), &doc.media, scene, doc.editor.state.frame, export::RenderSpec { scale, transparent: false }).map_err(|e| e.to_string())
}

/// Picks a library name that is free: `name`, then `name 2`, `name 3`, ...
fn free_name(doc: &anim_scene::Document, wanted: &str) -> String {
    if doc.library.find_index(wanted).is_none() {
        return wanted.to_owned();
    }
    let (stem, ext) = wanted.rsplit_once('.').map_or((wanted, ""), |(s, e)| (s, e));
    for n in 2..10_000 {
        let candidate = if ext.is_empty() { format!("{stem} {n}") } else { format!("{stem} {n}.{ext}") };
        if doc.library.find_index(&candidate).is_none() {
            return candidate;
        }
    }
    wanted.to_owned()
}

/// Imports artwork into the library (and onto the stage when `to_stage`). Returns the library name.
///
/// Raster images are stored as they are. PSD, AI/PDF and SVG files are rasterised at their own size (their layers are
/// flattened); the editor keeps no vector import yet.
///
/// # Errors
/// A message for unreadable or unsupported files.
pub fn import_artwork(doc: &mut AniDoc, path: &Path, to_stage: bool) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file_name = path.file_name().map_or_else(|| "image.png".to_owned(), |n| n.to_string_lossy().into_owned());
    let (data, name, compression, size) = if anim_artwork::detect(&bytes).is_some() {
        let tree = anim_artwork::import(&bytes, &anim_artwork::Limits::default()).map_err(|e| format!("{file_name}: {e}"))?;
        let mut store = anim_display::ImageStore::new();
        let list = anim_artwork::to_display(&tree, &mut store);
        let bounds = tree.root.bounds().unwrap_or_else(|| anim_core::Rect::new(0.0, 0.0, list.size.width.max(1.0), list.size.height.max(1.0)));
        let width = u32::try_from(num::ceil_f64_to_i32(bounds.width().max(1.0))).unwrap_or(1).clamp(1, 8192);
        let height = u32::try_from(num::ceil_f64_to_i32(bounds.height().max(1.0))).unwrap_or(1).clamp(1, 8192);
        let options = anim_render::RenderOptions::new(width, height)
            .with_view(Affine::translate((-bounds.x0, -bounds.y0)))
            .with_background(anim_render::Background::Transparent);
        let image = anim_render::render(&list, &store, &options).map_err(|e| e.to_string())?;
        let png = anim_export::encode_png(image.width, image.height, &anim_render::straight_rgba(&image)).map_err(|e| e.to_string())?;
        let stem = path.file_stem().map_or_else(|| "artwork".to_owned(), |s| s.to_string_lossy().into_owned());
        (png, format!("{stem}.png"), BitmapCompression::Lossless, [f64::from(image.width), f64::from(image.height)])
    } else {
        let image = anicraft_xfl::decode_bitmap_bytes(&bytes).map_err(|e| format!("{file_name}: {e}"))?;
        let jpeg = bytes.starts_with(&[0xFF, 0xD8]);
        (bytes, file_name.clone(), if jpeg { BitmapCompression::Photo } else { BitmapCompression::Lossless }, [f64::from(image.width), f64::from(image.height)])
    };
    let name = free_name(doc.document(), &name);
    let media = doc.media.insert(data).map_err(|e| e.to_string())?;
    let item_id = doc.document().library.fresh_item_id(&name);
    let mut item = BitmapItem::new(&name, item_id);
    item.href = Some(file_name);
    item.media = Some(media);
    item.compression = compression;
    item.size = Some(size);
    doc.editor.document.library.push(LibraryItem::Bitmap(item)).map_err(|e| e.to_string())?;
    if to_stage {
        place_bitmap(doc, &name, size)?;
    }
    Ok(name)
}

/// Puts a bitmap instance centred on the stage on the current layer and frame.
///
/// # Errors
/// When the layer or frame does not exist.
pub fn place_bitmap(doc: &mut AniDoc, name: &str, size: [f64; 2]) -> Result<(), String> {
    let id = doc.editor.state.timeline.clone();
    let (layer, frame) = (doc.editor.state.layer, doc.editor.state.frame);
    let (w, h) = (doc.document().width, doc.document().height);
    let matrix = Affine::translate(((w - size[0]) / 2.0, (h - size[1]) / 2.0));
    let at = anim_scene::edit::add_element(&mut doc.editor.document, &id, layer, frame, Element::Bitmap(BitmapInstance::new(name, matrix))).map_err(|e| e.to_string())?;
    let keyframe = doc.document().timeline(&id).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).map_or(0, |k| k.index);
    doc.editor.state.selection = anicraft_tools::Selection::one(anicraft_tools::ElementRef::top_level(layer, keyframe, at));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| [u8::try_from(i % 251).unwrap(), 80, 160, 255]).collect();
        anim_export::encode_png(w, h, &rgba).unwrap()
    }

    #[test]
    fn importing_a_png_adds_a_bitmap_and_an_instance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pic.png");
        std::fs::write(&path, png_bytes(40, 20)).unwrap();
        let mut doc = AniDoc::default();
        let name = import_artwork(&mut doc, &path, true).unwrap();
        assert_eq!(name, "pic.png");
        assert!(doc.document().library.find_bitmap("pic.png").is_some());
        let frame = doc.document().scenes[0].layers[0].keyframe_at(0).unwrap();
        assert!(matches!(frame.elements.first(), Some(Element::Bitmap(_))));
        // A second import of the same file gets a free name.
        assert_eq!(import_artwork(&mut doc, &path, false).unwrap(), "pic 2.png");
        // The document with the imported bitmap saves and reads back.
        let out = dir.path().join("out.fla");
        let mut file = File::create(&out).unwrap();
        write_fla(&doc, &mut file).unwrap();
        drop(file);
        let again = Project::open(&out).unwrap();
        assert!(again.document().library.find_bitmap("pic.png").is_some());
    }

    #[test]
    fn hostile_files_are_errors_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\nnot really").unwrap();
        let mut doc = AniDoc::default();
        assert!(import_artwork(&mut doc, &path, true).is_err());
        assert!(import_artwork(&mut doc, &dir.path().join("missing.png"), true).is_err());
        assert!(doc.document().library.items.is_empty());
    }

    #[test]
    fn svg_is_rasterised_on_import() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dot.svg");
        std::fs::write(&path, br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect width="40" height="30" fill="#f80"/></svg>"##).unwrap();
        let mut doc = AniDoc::default();
        let name = import_artwork(&mut doc, &path, false).unwrap();
        assert_eq!(name, "dot.png");
    }
}
