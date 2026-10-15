//! Reads Animate documents (zipped `.fla` and unzipped XFL folders) into the [`anim_scene`] model.
//!
//! ```no_run
//! let (document, media, warnings) = anicraft_xfl::read("movie.fla")?;
//! println!("{} scenes, {} library items, {} warnings", document.scenes.len(), document.library.items.len(), warnings.len());
//! # Ok::<(), anicraft_xfl::XflError>(())
//! ```
//!
//! The reader is tolerant and treats every byte as hostile: sizes are capped, nesting is bounded, and anything it
//! does not model is kept verbatim in the `extra` field of the struct it belongs to (attributes as name/value pairs,
//! child elements as raw XML) so that a writer can emit it again. [`Loaded::unknowns`] counts what ended up there.
//!
//! What `read` does, in order:
//! 1. opens the archive with [`anim_archive::Container`] (zip with recovery strategies, or a folder, or the parent
//!    folder of an `*.xfl` marker file);
//! 2. parses `DOMDocument.xml` and follows every `<Include href>` of `<symbols>` into `LIBRARY/`;
//! 3. loads the `bin/*.dat` blobs of bitmaps, sounds and videos into a [`MediaStore`];
//! 4. keeps `PublishSettings.xml` in the document and the other archive entries as [`Sidecar`]s.
//!
//! [`decode_bitmap`] turns a stored bitmap (Animate's lossless `.dat` format, or JPEG/PNG) into an
//! [`anim_display::Image`].

#![cfg_attr(
    test,
    allow(
        clippy::assert_is_empty,
        clippy::arithmetic_side_effects,
        clippy::field_reassign_with_default,
        reason = "assertions on emptiness, index arithmetic and default-then-tweak setup read better in tests"
    )
)]

pub mod diff;
pub mod document;
pub mod element;
pub mod error;
pub mod library;
pub mod media;
pub mod shape;
pub mod timeline;
pub mod write;
pub mod xml;

use std::path::Path;

use anim_archive::{Container, ContainerKind, IndexSource, Limits};
use anim_scene::{Document, MediaStore};
use serde::{Deserialize, Serialize};

pub use error::XflError;
pub use media::{BitmapFormat, LosslessHeader, decode_bitmap, decode_bitmap_bytes, encode_lossless_bitmap, lossless_header, sniff_bitmap, sound_payload};
pub use write::{WriteOptions, WriteReport, write_fla, write_fla_bytes, write_loaded, write_path, write_to, write_xfl_folder};
pub use xml::{Unknowns, Warning};

/// Parser limit on XML nodes per file.
pub const MAX_XML_NODES: u32 = 50_000_000;

/// How [`read_with`] behaves.
#[derive(Debug, Clone)]
pub struct ReadOptions {
    /// Load the `bin/*.dat` blobs (bitmaps, sounds, video). Off makes opening fast for inspection.
    pub load_media: bool,
    /// Caps for the archive reader.
    pub limits: Limits,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self { load_media: true, limits: Limits::default() }
    }
}

/// An archive entry the document model has no slot for, kept so a writer can put it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Entry name, for example `META-INF/metadata.xml`.
    pub name: String,
    pub data: Vec<u8>,
}

/// How the archive was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInfo {
    pub folder: bool,
    /// The zip index strategy that worked (`None` for folders).
    pub recovered_index: bool,
}

/// Everything [`read_with`] returns.
#[derive(Debug)]
pub struct Loaded {
    pub document: Document,
    pub media: MediaStore,
    pub warnings: Vec<Warning>,
    /// Warnings dropped after the first couple of thousand.
    pub dropped_warnings: u64,
    /// What was kept in `extra`, by `Tag@attribute` and `Parent/Child`.
    pub unknowns: Unknowns,
    /// Small non-model entries: `META-INF/metadata.xml`, `MobileSettings.xml`, the `*.xfl` marker, `bin/SymDepend.cache`.
    pub sidecars: Vec<Sidecar>,
    pub source: SourceInfo,
}

/// A JSON-friendly digest of a loaded document, for command-line tools and tool servers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSummary {
    pub width: f64,
    pub height: f64,
    pub frame_rate: f64,
    pub scenes: u32,
    pub total_frames: u32,
    pub layers: u32,
    pub keyframes: u32,
    pub elements: u32,
    pub symbols: u32,
    pub bitmaps: u32,
    pub sounds: u32,
    pub fonts: u32,
    pub videos: u32,
    pub folders: u32,
    /// Number of distinct blobs in the media store and their total size.
    pub media_blobs: usize,
    pub media_bytes: u64,
    pub warnings: usize,
    pub dropped_warnings: u64,
    /// Unknown attributes and elements kept in `extra`, and the most frequent ones.
    pub unknown_total: u64,
    pub top_unknowns: Vec<(String, u64)>,
    pub folder: bool,
    pub recovered_index: bool,
}

impl Loaded {
    /// Counts and the `top` most frequent unknowns, as plain serialisable data.
    #[must_use]
    pub fn summary(&self, top: usize) -> LoadSummary {
        let stats = self.document.stats();
        LoadSummary {
            width: self.document.width,
            height: self.document.height,
            frame_rate: self.document.frame_rate,
            scenes: stats.scenes,
            total_frames: stats.total_frames,
            layers: stats.layers,
            keyframes: stats.keyframes,
            elements: stats.elements,
            symbols: stats.symbols,
            bitmaps: stats.bitmaps,
            sounds: stats.sounds,
            fonts: stats.fonts,
            videos: stats.videos,
            folders: stats.folders,
            media_blobs: self.media.len(),
            media_bytes: self.media.total_bytes(),
            warnings: self.warnings.len(),
            dropped_warnings: self.dropped_warnings,
            unknown_total: self.unknowns.total(),
            top_unknowns: self.unknowns.top(top),
            folder: self.source.folder,
            recovered_index: self.source.recovered_index,
        }
    }
}

/// Entries kept as sidecars when they are at most this large.
const MAX_SIDECAR_BYTES: u64 = 4 * 1024 * 1024;

/// Reads a `.fla`, an XFL folder or an `*.xfl` marker file.
///
/// # Errors
/// [`XflError`] when the archive cannot be opened or has no valid `DOMDocument.xml`. Damage inside the document is
/// reported as warnings instead.
pub fn read(path: impl AsRef<Path>) -> Result<(Document, MediaStore, Vec<Warning>), XflError> {
    let loaded = read_with(path, &ReadOptions::default())?;
    Ok((loaded.document, loaded.media, loaded.warnings))
}

/// Like [`read`], with options and the full result.
///
/// # Errors
/// See [`read`].
pub fn read_with(path: impl AsRef<Path>, options: &ReadOptions) -> Result<Loaded, XflError> {
    let container = Container::open_with(path.as_ref(), options.limits)?;
    read_container(&container, options)
}

/// Reads a document held in memory (a zipped `.fla`).
///
/// # Errors
/// See [`read`].
pub fn read_bytes(bytes: Vec<u8>, options: &ReadOptions) -> Result<Loaded, XflError> {
    let container = Container::from_bytes_with(bytes, options.limits)?;
    read_container(&container, options)
}

/// Reads a document from an already opened container.
///
/// # Errors
/// See [`read`].
pub fn read_container(container: &Container, options: &ReadOptions) -> Result<Loaded, XflError> {
    let mut sink = xml::Sink::default();
    for message in container.warnings() {
        sink.warn("archive", message.clone());
    }
    let main = container.find("DOMDocument.xml").ok_or(XflError::MissingDocument)?.name.clone();
    let text = container.read_text(&main)?;
    let xml = xml::parse(&text).map_err(|failure| match failure {
        xml::ParseFailure::Xml(source) => XflError::Xml { file: main.clone(), source },
        xml::ParseFailure::TooDeep => XflError::TooDeep(main.clone()),
    })?;
    let root = xml.root_element();
    if root.tag_name().name() != "DOMDocument" {
        return Err(XflError::NotXfl(root.tag_name().name().to_owned()));
    }
    let mut media = MediaStore::new();
    let mut document = {
        let mut ctx = xml::Ctx::new(&text, &main, &mut sink);
        document::read_document(&mut ctx, root, container, &mut media, options.load_media)
    };
    document.publish_settings_xml = container.find("PublishSettings.xml").and_then(|entry| match container.read_text(&entry.name) {
        Ok(text) => Some(text),
        Err(error) => {
            sink.warn(&entry.name, format!("cannot read: {error}"));
            None
        }
    });
    let sidecars = collect_sidecars(container, &mut sink);
    let source = SourceInfo {
        folder: container.kind() == ContainerKind::Folder,
        recovered_index: !matches!(container.index_source(), IndexSource::ZipCrate | IndexSource::Folder),
    };
    Ok(Loaded { document, media, warnings: sink.warnings, dropped_warnings: sink.dropped_warnings, unknowns: sink.unknowns, sidecars, source })
}

/// Original media files Animate keeps next to the symbols (`LIBRARY/song.wav`, `LIBRARY/clip.mp3`) are kept as
/// sidecars when each is at most this large...
const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// ... and while they total at most this much.
const MAX_SOURCES_TOTAL: u64 = 512 * 1024 * 1024;

fn collect_sidecars(container: &Container, sink: &mut xml::Sink) -> Vec<Sidecar> {
    let mut out = Vec::new();
    let mut sources_total = 0u64;
    for entry in container.files() {
        let lower = entry.name.to_ascii_lowercase();
        let marker = !lower.contains('/') && Path::new(&lower).extension().is_some_and(|e| e.eq_ignore_ascii_case("xfl"));
        let is_small = marker || matches!(lower.as_str(), "meta-inf/metadata.xml" | "mobilesettings.xml" | "bin/symdepend.cache");
        let is_source = lower.starts_with("library/") && !Path::new(&lower).extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"));
        if is_source {
            let fits = entry.size <= MAX_SOURCE_BYTES && sources_total.saturating_add(entry.size) <= MAX_SOURCES_TOTAL;
            if !fits {
                continue;
            }
            sources_total = sources_total.saturating_add(entry.size);
        } else if !is_small || entry.size > MAX_SIDECAR_BYTES {
            continue;
        }
        match container.read_entry(entry) {
            Ok(data) => out.push(Sidecar { name: entry.name.clone(), data }),
            Err(error) => sink.warn(&entry.name, format!("cannot read: {error}")),
        }
    }
    out
}
