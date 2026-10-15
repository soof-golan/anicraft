//! Opening a document and getting frames out of it: the glue between `anicraft-xfl` and `anim-timeline`.
//!
//! Frame numbers shown to people (command line, MCP) are 1-based like Animate's timeline; everything inside the
//! engine is 0-based. The conversion happens only in [`resolve_span`] and [`one_based`].

use std::ops::Range;
use std::path::{Path, PathBuf};

use anicraft_xfl::{Loaded, ReadOptions};
use anim_core::num;
use anim_display::Image;
use anim_scene::{Document, MediaStore};
use anim_timeline::{EvalOptions, Evaluator, ImageProvider};

use crate::error::{CliError, Result};

/// Most frames one export may produce, so a typo in `--fps` cannot allocate forever.
pub const MAX_OUTPUT_FRAMES: usize = 200_000;

/// Decodes the bitmaps stored in an Animate document (lossless `.dat`, JPEG, PNG) for the timeline evaluator.
#[derive(Debug, Clone, Copy, Default)]
pub struct XflBitmaps;

impl ImageProvider for XflBitmaps {
    fn decode(&mut self, _name: &str, bytes: &[u8]) -> Option<Image> {
        anicraft_xfl::decode_bitmap_bytes(bytes).ok()
    }
}

/// A document read from disk together with everything needed to evaluate and write it back.
#[derive(Debug)]
pub struct Project {
    /// Where it was read from.
    pub path: PathBuf,
    /// Document, media, warnings and sidecars as the reader returned them.
    pub loaded: Loaded,
}

impl Project {
    /// Reads a `.fla`, an XFL folder or an `.xfl` marker.
    ///
    /// # Errors
    /// [`CliError::Xfl`] when the file cannot be read as an Animate document.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let loaded = anicraft_xfl::read_with(path, &ReadOptions::default())?;
        Ok(Self { path: path.to_path_buf(), loaded })
    }

    /// The document model.
    #[must_use]
    pub const fn document(&self) -> &Document {
        &self.loaded.document
    }

    /// Bitmaps, sounds and video bytes.
    #[must_use]
    pub const fn media(&self) -> &MediaStore {
        &self.loaded.media
    }

    /// An evaluator over this project. One evaluator per document revision keeps decoded bitmaps cached.
    /// `playing` advances movie clips with the timeline, which is what export wants.
    #[must_use]
    pub fn evaluator(&self, playing: bool) -> Evaluator<'_> {
        evaluator_for(self.document(), self.media(), playing)
    }

    /// The scene named `name`, or the one the document had open (usually the first).
    ///
    /// # Errors
    /// [`CliError::Usage`] for an unknown scene name, listing the available ones.
    pub fn scene_index(&self, name: Option<&str>) -> Result<usize> {
        scene_index(self.document(), name)
    }
}

/// An evaluator over a document and its media, decoding Animate bitmaps.
#[must_use]
pub fn evaluator_for<'a>(doc: &'a Document, media: &'a MediaStore, playing: bool) -> Evaluator<'a> {
    Evaluator::new(doc).with_media(media).with_provider(XflBitmaps).with_options(EvalOptions { advance_movie_clips: playing, ..EvalOptions::default() })
}

/// Picks a scene by name (`None` = the document's current scene, clamped to the available ones).
///
/// # Errors
/// [`CliError::Usage`] for an unknown name or a document without scenes.
pub fn scene_index(doc: &Document, name: Option<&str>) -> Result<usize> {
    if doc.scenes.is_empty() {
        return Err(CliError::usage("the document has no scenes"));
    }
    let Some(wanted) = name else {
        return Ok(doc.current_timeline.min(doc.scenes.len().saturating_sub(1)));
    };
    doc.scene_index(wanted).ok_or_else(|| {
        let names: Vec<&str> = doc.scenes.iter().map(|s| s.name.as_str()).collect();
        CliError::usage(format!("no scene named {wanted:?}; the document has: {}", names.join(", ")))
    })
}

/// The 0-based half-open frame range for 1-based inclusive `from` and `to` (defaults: the whole scene).
///
/// # Errors
/// [`CliError::Usage`] for frame 0, a start beyond the end, or a start outside the scene.
pub fn resolve_span(total: u32, from: Option<u32>, to: Option<u32>) -> Result<Range<u32>> {
    if total == 0 {
        return Err(CliError::usage("the scene has no frames"));
    }
    let first = from.unwrap_or(1);
    let last = to.unwrap_or(total).min(total);
    if first == 0 || to == Some(0) {
        return Err(CliError::usage("frame numbers start at 1"));
    }
    if first > total {
        return Err(CliError::usage(format!("frame {first} is beyond the end of the scene ({total} frames)")));
    }
    if last < first {
        return Err(CliError::usage(format!("--to {last} is before --from {first}")));
    }
    Ok(first.saturating_sub(1)..last)
}

/// The 1-based number of a 0-based frame.
#[must_use]
pub const fn one_based(frame: u32) -> u32 {
    frame.saturating_add(1)
}

/// The document frames to show for each output frame.
///
/// Without `fps`, or at the document's own rate, that is every frame of `span`; at another rate output frame `k`
/// shows the document frame playing at `k / fps` seconds.
///
/// # Errors
/// [`CliError::Usage`] for a rate that is not positive and finite, or one that would make more than
/// [`MAX_OUTPUT_FRAMES`] frames.
pub fn frame_plan(span: &Range<u32>, doc_fps: f64, fps: Option<f64>) -> Result<Vec<u32>> {
    let count = span.end.saturating_sub(span.start);
    let Some(out_fps) = fps else {
        return Ok(span.clone().collect());
    };
    if !(out_fps.is_finite() && out_fps > 0.0 && out_fps <= 1000.0) {
        return Err(CliError::usage(format!("--fps {out_fps} is not between 0 and 1000")));
    }
    if !(doc_fps.is_finite() && doc_fps > 0.0) || (out_fps - doc_fps).abs() < 1e-9 {
        return Ok(span.clone().collect());
    }
    let seconds = f64::from(count) / doc_fps;
    let wanted = num::round_f64_to_u32(seconds * out_fps).max(1);
    if num::u32_to_usize(wanted) > MAX_OUTPUT_FRAMES {
        return Err(CliError::usage(format!("{wanted} output frames is more than the limit of {MAX_OUTPUT_FRAMES}")));
    }
    let last = span.end.saturating_sub(1);
    Ok((0..wanted)
        .map(|k| {
            let offset = u32::try_from(num::floor_f64_to_i32(f64::from(k) * doc_fps / out_fps)).unwrap_or(0);
            span.start.saturating_add(offset).min(last)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_is_one_based_and_inclusive() {
        assert_eq!(resolve_span(10, None, None).unwrap(), 0..10);
        assert_eq!(resolve_span(10, Some(3), Some(5)).unwrap(), 2..5);
        assert_eq!(resolve_span(10, Some(10), Some(99)).unwrap(), 9..10);
        assert!(resolve_span(10, Some(0), None).is_err());
        assert!(resolve_span(10, Some(11), None).is_err());
        assert!(resolve_span(10, Some(5), Some(4)).is_err());
        assert!(resolve_span(0, None, None).is_err());
    }

    #[test]
    fn plan_resamples_by_time() {
        assert_eq!(frame_plan(&(0..4), 24.0, None).unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(frame_plan(&(0..4), 24.0, Some(24.0)).unwrap(), vec![0, 1, 2, 3]);
        assert_eq!(frame_plan(&(0..4), 12.0, Some(24.0)).unwrap(), vec![0, 0, 1, 1, 2, 2, 3, 3]);
        assert_eq!(frame_plan(&(2..6), 24.0, Some(12.0)).unwrap(), vec![2, 4]);
        assert!(frame_plan(&(0..4), 24.0, Some(0.0)).is_err());
        assert!(frame_plan(&(0..4), 24.0, Some(f64::NAN)).is_err());
    }

    #[test]
    fn unknown_scene_lists_the_known_ones() {
        let doc = Document::default();
        let err = scene_index(&doc, Some("nope")).unwrap_err().to_string();
        assert!(err.contains("Scene 1"), "{err}");
        assert_eq!(scene_index(&doc, None).unwrap(), 0);
    }
}
