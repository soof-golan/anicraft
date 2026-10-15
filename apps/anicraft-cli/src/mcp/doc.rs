//! The document an MCP session edits: the model plus the bytes it refers to.

use anicraft_xfl::Sidecar;
use anim_scene::{Document, MediaStore};
use anim_session::SizeHint;

use crate::project::Project;

/// What the session holds: the model, its media and the archive entries the model has no slot for.
///
/// Cloned for every undo step, so the history budget counts the media bytes ([`SizeHint`]).
#[derive(Debug, Clone)]
pub struct AniDoc {
    /// The document model.
    pub document: Document,
    /// Bitmaps, sounds and video.
    pub media: MediaStore,
    /// `META-INF/metadata.xml` and friends, written back on save.
    pub sidecars: Vec<Sidecar>,
}

impl Default for AniDoc {
    fn default() -> Self {
        Self { document: Document::default(), media: MediaStore::new(), sidecars: Vec::new() }
    }
}

impl SizeHint for AniDoc {
    fn size_hint(&self) -> usize {
        let media = usize::try_from(self.media.total_bytes()).unwrap_or(usize::MAX);
        let model = usize::try_from(self.document.keyframe_count()).unwrap_or(usize::MAX).saturating_mul(512);
        media.saturating_add(model)
    }
}

impl From<Project> for AniDoc {
    fn from(project: Project) -> Self {
        let loaded = project.loaded;
        Self { document: loaded.document, media: loaded.media, sidecars: loaded.sidecars }
    }
}
