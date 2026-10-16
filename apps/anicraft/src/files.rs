//! Files: opening, saving, importing artwork and exporting. Everything that touches the disk lives here so the window
//! code stays about state.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anicraft_cli::export::{self, ExportJob, ExportOutcome, Format};
use anicraft_cli::mcp::AniDoc;
use anicraft_cli::project::{Project, evaluator_for, resolve_span};
use anicraft_xfl::WriteOptions;
use anim_scene::TimelineId;
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
        let span = if request.format == Format::Svg {
            let at = request.frame.min(total.saturating_sub(1));
            at..at.saturating_add(1)
        } else {
            resolve_span(total, None, None).map_err(|e| JobError::Failed(e.to_string()))?
        };
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

pub use anicraft_cli::import::{import_artwork, place_bitmap};

#[cfg(test)]
mod tests {
    use anim_scene::Element;

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
