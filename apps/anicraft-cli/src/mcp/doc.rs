//! The document an MCP session edits: the editor (model, selection, tools) plus the bytes the model refers to.

use anicraft_tools::{Editor, ToolHost};
use anicraft_xfl::Sidecar;
use anim_scene::{Document, MediaStore};
use anim_session::SizeHint;

use crate::project::Project;

/// What the session holds: the editor, the media and the archive entries the model has no slot for.
///
/// Cloned for every undo step, so the history budget counts the media bytes ([`SizeHint`]). The editor does not keep
/// its own undo stack; the session snapshots this whole value. The GUI application uses the same type, so a running
/// editor and the headless server expose the same commands.
#[derive(Debug, Clone)]
pub struct AniDoc {
    /// The document model with the editing state (selection, active tool, playhead) and the tools.
    pub editor: Editor,
    /// Bitmaps, sounds and video.
    pub media: MediaStore,
    /// `META-INF/metadata.xml` and friends, written back on save.
    pub sidecars: Vec<Sidecar>,
}

impl AniDoc {
    /// A session document over `document` and its media.
    #[must_use]
    pub fn new(document: Document, media: MediaStore, sidecars: Vec<Sidecar>) -> Self {
        let mut editor = Editor::new(document);
        editor.keep_history = false;
        Self { editor, media, sidecars }
    }

    /// The document model.
    #[must_use]
    pub const fn document(&self) -> &Document {
        &self.editor.document
    }

    /// The document model, mutably.
    pub const fn document_mut(&mut self) -> &mut Document {
        &mut self.editor.document
    }
}

impl Default for AniDoc {
    fn default() -> Self {
        Self::new(Document::default(), MediaStore::new(), Vec::new())
    }
}

impl SizeHint for AniDoc {
    fn size_hint(&self) -> usize {
        let media = usize::try_from(self.media.total_bytes()).unwrap_or(usize::MAX);
        let model = usize::try_from(self.editor.document.keyframe_count()).unwrap_or(usize::MAX).saturating_mul(512);
        media.saturating_add(model)
    }
}

impl ToolHost for AniDoc {
    fn editor(&self) -> &Editor {
        &self.editor
    }

    fn editor_mut(&mut self) -> &mut Editor {
        &mut self.editor
    }
}

impl From<Project> for AniDoc {
    fn from(project: Project) -> Self {
        let loaded = project.loaded;
        Self::new(loaded.document, loaded.media, loaded.sidecars)
    }
}
