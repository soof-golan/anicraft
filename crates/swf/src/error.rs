//! Errors of the SWF writer.

use anim_display::ImageId;

/// Everything that can go wrong while building a SWF.
#[derive(Debug, thiserror::Error)]
pub enum SwfError {
    /// A SWF has at most 65 535 characters (shapes, bitmaps, sprites, sounds).
    #[error("more than 65535 characters in one SWF")]
    TooManyCharacters,
    /// Depths are 16-bit.
    #[error("display list depth {0} does not fit in 16 bits")]
    DepthOutOfRange(usize),
    /// A tag body, or the whole file, does not fit the 32-bit length fields.
    #[error("tag or file of {0} bytes exceeds the 32-bit length limit")]
    TooLarge(usize),
    /// A bitmap dimension does not fit the 16-bit fields of `DefineBitsLossless2`.
    #[error("bitmap of {width}x{height} exceeds the 65535 pixel limit")]
    BitmapTooLarge { width: u32, height: u32 },
    /// A bitmap buffer is shorter than its dimensions require.
    #[error("bitmap of {width}x{height} needs {expected} bytes but has {actual}")]
    BitmapSize { width: u32, height: u32, expected: usize, actual: usize },
    /// A display list refers to an image the store does not hold.
    #[error("image {0:?} is missing from the image store")]
    MissingImage(ImageId),
    /// SWF strings are NUL-terminated and cannot contain NUL.
    #[error("string contains a NUL character")]
    InteriorNul,
    /// A shape edge is longer than 17 signed bits of twips; the shape builder splits long edges, so this only
    /// happens when records are assembled by hand.
    #[error("shape edge delta {0} does not fit the 17-bit SWF limit")]
    EdgeTooLong(i32),
    /// More fill or line styles than the 15-bit style index of a shape record can address.
    #[error("{0} styles in one shape (at most 32767)")]
    TooManyStyles(usize),
    /// An object carries more than 255 filters.
    #[error("{0} filters on one object (at most 255)")]
    TooManyFilters(usize),
    /// A timeline holds more than 65535 frames.
    #[error("more than 65535 frames in one timeline")]
    TooManyFrames,
    /// Groups nested deeper than the exporter follows.
    #[error("display list nested deeper than {0} levels")]
    TooDeep(usize),
    /// The MP3 stream is unusable.
    #[error("invalid MP3 stream: {0}")]
    InvalidMp3(&'static str),
    /// Two shapes cannot be the ends of one shape tween.
    #[error("cannot morph between these shapes: {0}")]
    MorphMismatch(&'static str),
    /// A PCM sound format SWF cannot play without resampling.
    #[error("unsupported sound: {0}")]
    UnsupportedSound(&'static str),
    /// The frame rate is not a positive finite number below 256 frames per second.
    #[error("frame rate {0} is outside the SWF range of 0..256 frames per second")]
    FrameRate(f64),
    /// The stage size is not a positive finite number of pixels.
    #[error("stage size {width}x{height} is not valid")]
    StageSize { width: f64, height: f64 },
    /// Compressing the body failed.
    #[error("compression failed: {0}")]
    Compression(#[from] std::io::Error),
}
