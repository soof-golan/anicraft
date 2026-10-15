//! Errors of the XFL reader.

use anim_archive::ArchiveError;
use anim_display::ImageError;
use anim_scene::{MediaId, SceneError};

/// Why a document, or a piece of media, could not be read.
#[derive(Debug, thiserror::Error)]
pub enum XflError {
    #[error(transparent)]
    Archive(#[from] ArchiveError),
    #[error("the archive has no DOMDocument.xml")]
    MissingDocument,
    #[error("{file}: invalid XML: {source}")]
    Xml {
        file: String,
        #[source]
        source: roxmltree::Error,
    },
    #[error("DOMDocument.xml starts with <{0}>, not <DOMDocument>")]
    NotXfl(String),
    #[error(transparent)]
    Scene(#[from] SceneError),
    #[error("media {0:?} is not in the store")]
    NoSuchMedia(MediaId),
    #[error("bitmap data is not understood: {0}")]
    Bitmap(String),
    #[error(transparent)]
    Image(#[from] ImageError),
}
