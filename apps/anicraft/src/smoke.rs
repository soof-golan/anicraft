//! `anicraft --smoke [FILE]`: load a document (or a new one), render a frame offscreen with the editor's own render
//! path, and report. No window is opened, so CI and the QA task can run it anywhere.

use std::path::Path;

use anicraft_cli::mcp::AniDoc;
use anicraft_cli::project::{Project, evaluator_for};
use anim_core::num;
use serde::Serialize;

use crate::worker::{RenderJob, Target, render_job};

/// What the smoke run found.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SmokeReport {
    pub file: Option<String>,
    pub scenes: usize,
    pub layers: usize,
    pub library_items: usize,
    pub scene_frames: u32,
    /// The 1-based frame that was rendered.
    pub frame: u32,
    pub width: u32,
    pub height: u32,
    /// Fraction of pixels that differ from the first pixel: 0 for a blank frame.
    pub ink: f64,
    /// Commands the editor registers (menu items, tools and document commands).
    pub commands: usize,
}

/// Loads, renders and summarises.
///
/// `frame` is 1-based; `None` renders the first frame that has any content (so a clip that starts late still
/// shows something), falling back to frame 1.
///
/// # Errors
/// A readable message for unreadable documents or render failures.
pub fn run(file: Option<&Path>, frame: Option<u32>) -> Result<SmokeReport, String> {
    let doc = match file {
        Some(path) => AniDoc::from(Project::open(path).map_err(|e| format!("{}: {e}", path.display()))?),
        None => AniDoc::default(),
    };
    let document = doc.document();
    let scene = document.current_timeline.min(document.scenes.len().saturating_sub(1));
    let mut ev = evaluator_for(document, &doc.media, false);
    let total = ev.scene_frames(scene);
    let wanted = frame.map_or_else(|| first_frame_with_content(document, scene), |f| f.saturating_sub(1));
    let chosen = wanted.min(total.saturating_sub(1));
    let width = num::ceil_f64_to_i32(document.width.clamp(1.0, 1920.0));
    let height = num::ceil_f64_to_i32((document.height * (f64::from(width) / document.width.max(1.0))).clamp(1.0, 4096.0));
    let job = RenderJob {
        id: 1,
        target: Target::Scene(scene),
        frame: chosen,
        size: [u32::try_from(width).unwrap_or(1), u32::try_from(height).unwrap_or(1)],
        onion: None,
        playing: false,
        transparent: false,
    };
    let image = render_job(&mut ev, &job)?;
    let first = image.premultiplied_rgba.chunks_exact(4).next().map(<[u8]>::to_vec).unwrap_or_default();
    let different = image.premultiplied_rgba.chunks_exact(4).filter(|px| *px != first.as_slice()).count();
    let pixels = image.premultiplied_rgba.len() / 4;
    Ok(SmokeReport {
        file: file.map(|p| p.display().to_string()),
        scenes: document.scenes.len(),
        layers: document.scenes.iter().map(|s| s.layers.len()).sum(),
        library_items: document.library.items.len(),
        scene_frames: total,
        frame: chosen.saturating_add(1),
        width: image.width,
        height: image.height,
        ink: if pixels == 0 { 0.0 } else { num::usize_to_f64(different) / num::usize_to_f64(pixels) },
        commands: crate::commands::registry().iter().count(),
    })
}

/// The first frame (0-based) of the scene where any layer has an element.
fn first_frame_with_content(doc: &anim_scene::Document, scene: usize) -> u32 {
    doc.scenes
        .get(scene)
        .and_then(|t| t.layers.iter().flat_map(|l| l.frames.iter()).filter(|f| !f.elements.is_empty() || f.is_tweened()).map(|f| f.index).min())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_document_renders_blank() {
        let report = run(None, None).unwrap();
        assert_eq!((report.width, report.height), (550, 400));
        assert!(report.ink.abs() < f64::EPSILON);
        assert!(report.commands > 40, "{} commands", report.commands);
    }

    #[test]
    fn a_missing_file_is_a_clean_error() {
        let err = run(Some(Path::new("/definitely/not/here.fla")), None).unwrap_err();
        assert!(err.contains("here.fla"), "{err}");
    }

    #[test]
    fn the_cli_fixture_renders_ink() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../anicraft-cli/tests/fixtures/mover");
        if !fixture.exists() {
            return;
        }
        let report = run(Some(&fixture), Some(6)).unwrap();
        assert_eq!(report.frame, 6);
        assert!(report.ink > 0.0, "{report:?}");
    }
}
