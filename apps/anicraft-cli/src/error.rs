//! Errors of the command-line tool.

use std::path::PathBuf;

/// Everything that can go wrong in a command. The binary prints these through `color-eyre`.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// The document could not be read or written.
    #[error(transparent)]
    Xfl(#[from] anicraft_xfl::XflError),
    /// A frame could not be rasterised.
    #[error("render failed: {0}")]
    Render(#[from] anim_render::RenderError),
    /// An exporter failed.
    #[error("export failed: {0}")]
    Export(#[from] anim_export::ExportError),
    /// The video encoder failed.
    #[error("video encoding failed: {0}")]
    Video(#[from] anim_video::VideoError),
    /// The SWF exporter failed.
    #[error("swf export failed: {0}")]
    Swf(#[from] anicraft_swf::SwfError),
    /// Audio could not be decoded or mixed.
    #[error("audio failed: {0}")]
    Audio(#[from] anim_audio::AudioError),
    /// A file or folder could not be read or written.
    #[error("{path}: {source}")]
    Io {
        /// The path involved.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The command line asked for something that cannot be done.
    #[error("{0}")]
    Usage(String),
}

impl CliError {
    /// A usage error with this message.
    #[must_use]
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage(message.into())
    }

    /// An I/O error for `path`.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }
}

/// Result alias of the crate.
pub type Result<T> = std::result::Result<T, CliError>;
