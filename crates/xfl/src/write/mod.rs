//! Writes a [`Document`] and its [`MediaStore`] as an XFL folder or a zipped `.fla`.
//!
//! ```no_run
//! let loaded = anicraft_xfl::read_with("movie.fla", &anicraft_xfl::ReadOptions::default())?;
//! let report = anicraft_xfl::write::write_loaded("copy.fla", &loaded)?;
//! println!("{} entries, {} bytes", report.entries, report.bytes);
//! # Ok::<(), anicraft_xfl::XflError>(())
//! ```
//!
//! # Layout
//!
//! | entry | content |
//! |-------|---------|
//! | `mimetype` | `application/vnd.adobe.xfl`, stored, first entry of a zip |
//! | `NAME.xfl` | the marker file, `PROXY-CS5` |
//! | `DOMDocument.xml` | stage settings, library sections, scenes |
//! | `LIBRARY/...xml` | one file per symbol, named by its `Include href` |
//! | `bin/*.dat` | bitmaps, sounds and video, named by the items' `*DataHRef` |
//! | `PublishSettings.xml` | [`Document::publish_settings_xml`] |
//! | `MobileSettings.xml`, `META-INF/metadata.xml` | from the sidecars, else defaults |
//!
//! # Fidelity
//!
//! * Everything the model holds is written, and so is everything it parked in `extra`: unmodelled attributes in
//!   their original order (after the modelled ones), unmodelled child elements verbatim.
//! * The output is deterministic: the same document always produces the same bytes (the zip uses fixed time stamps).
//! * Media bytes are written as they are. A lossless bitmap item whose bytes are another image format (a PNG that
//!   was imported, say) is converted to Animate's lossless `.dat`; see [`crate::encode_lossless_bitmap`].
//! * `bin/SymDepend.cache` is a cache of Animate's own and goes stale as soon as symbols change, so it is only
//!   written back when [`WriteOptions::keep_caches`] is set.
//! * Attributes of the value types without an `extra` (`Matrix`, `SolidColor`, ...) that the reader dropped stay
//!   dropped. XML that is not well formed is never written: kept fragments are checked and left out with a warning.

mod library;
mod out;
mod scene;
mod shape;

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anim_archive::names::is_safe_relative;
use anim_archive::{ArchiveError, ArchiveWriter, DirectoryWriter, NewEntry, XFL_MIMETYPE, ZipWriter};
use anim_scene::{Document, MediaStore, Symbol};

pub use out::{escape_attr, escape_text, fmt_f32, fmt_f64, raw_is_element};

use crate::error::XflError;
use crate::xml::Warning;
use crate::{Loaded, Sidecar};

/// The marker file's content.
const MARKER: &str = "PROXY-CS5";
const DEFAULT_METADATA: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 5.6-c015 81.157285, 2014/12/12-00:43:15"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about=""/></rdf:RDF></x:xmpmeta>"#;
const DEFAULT_PUBLISH_SETTINGS: &str = "<flash_profiles>\n<flash_profile version=\"1.0\" name=\"Default\" current=\"true\">\n<PublishFormatProperties enabled=\"true\">\n<defaultNames>1</defaultNames>\n<flash>1</flash>\n</PublishFormatProperties>\n</flash_profile>\n</flash_profiles>\n";

/// How a document is written.
#[derive(Debug, Clone)]
pub struct WriteOptions {
    /// The name of the marker file without `.xfl` (a marker among the sidecars wins).
    pub name: String,
    /// Write `bin/SymDepend.cache` from the sidecars back too. Off by default: it describes the symbols of the file it
    /// was read from, and Animate rebuilds it.
    pub keep_caches: bool,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self { name: "document".to_owned(), keep_caches: false }
    }
}

/// What was written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteReport {
    /// Archive entries written, `mimetype` included for zips.
    pub entries: usize,
    /// Entries under `bin/`.
    pub media_entries: usize,
    /// Symbol files under `LIBRARY/`.
    pub symbols: usize,
    /// Uncompressed size of everything written.
    pub bytes: u64,
    /// Content that had to be changed or left out.
    pub warnings: Vec<Warning>,
}

fn warn(report: &mut WriteReport, file: &str, message: String) {
    report.warnings.push(Warning { file: file.to_owned(), message });
}

/// The text of `DOMDocument.xml` for `doc`, with the warnings that came up. Symbols and media are named as
/// [`write_to`] would name them.
#[must_use]
pub fn document_xml(doc: &Document, store: &MediaStore) -> (String, Vec<String>) {
    let plan = library::Plan::new(doc, store);
    let (xml, mut warnings) = plan.document_xml();
    warnings.extend(plan.warnings.iter().cloned());
    (xml, warnings)
}

/// The text of one symbol file.
#[must_use]
pub fn symbol_xml(symbol: &Symbol) -> (String, Vec<String>) {
    library::symbol_xml(symbol)
}

struct Names {
    seen: HashSet<String>,
}

impl Names {
    fn claim(&mut self, name: &str) -> bool {
        self.seen.insert(anim_archive::names::lookup_key(name))
    }
}

fn add(writer: &mut impl ArchiveWriter, names: &mut Names, report: &mut WriteReport, name: &str, data: &[u8]) -> Result<(), XflError> {
    if !is_safe_relative(name) {
        warn(report, name, "unsafe entry name, left out".to_owned());
        return Ok(());
    }
    if !names.claim(name) {
        warn(report, name, "duplicate entry name, left out".to_owned());
        return Ok(());
    }
    writer.add_entry(NewEntry::file(name, data))?;
    report.entries = report.entries.saturating_add(1);
    report.bytes = report.bytes.saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
    Ok(())
}

fn sidecar<'a>(sidecars: &'a [Sidecar], name: &str) -> Option<&'a Sidecar> {
    sidecars.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

/// Writes every entry of the document to `writer`.
///
/// `sidecars` are the small non-model entries a reader returned ([`Loaded::sidecars`]); pass an empty slice for a
/// new document. A zip should be created with `mimetype` as its first entry (the file functions do that).
///
/// # Errors
/// [`XflError::Archive`] when the destination cannot be written.
pub fn write_to(
    writer: &mut impl ArchiveWriter,
    doc: &Document,
    store: &MediaStore,
    sidecars: &[Sidecar],
    options: &WriteOptions,
) -> Result<WriteReport, XflError> {
    let mut report = WriteReport::default();
    let mut names = Names { seen: HashSet::new() };
    let plan = library::Plan::new(doc, store);
    for message in &plan.warnings {
        warn(&mut report, "bin", message.clone());
    }

    // The marker: the sidecar's name and content if there is one.
    let marker = sidecars.iter().find(|s| !s.name.contains('/') && Path::new(&s.name).extension().is_some_and(|e| e.eq_ignore_ascii_case("xfl")));
    let marker_name = marker.map_or_else(|| format!("{}.xfl", options.name), |m| m.name.clone());
    add(writer, &mut names, &mut report, &marker_name, marker.map_or(MARKER.as_bytes(), |m| m.data.as_slice()))?;

    let (dom, dom_warnings) = plan.document_xml();
    for message in dom_warnings {
        warn(&mut report, "DOMDocument.xml", message);
    }
    add(writer, &mut names, &mut report, "DOMDocument.xml", dom.as_bytes())?;
    drop(dom);

    let publish = doc.publish_settings_xml.as_deref().unwrap_or(DEFAULT_PUBLISH_SETTINGS);
    add(writer, &mut names, &mut report, "PublishSettings.xml", publish.as_bytes())?;
    add(writer, &mut names, &mut report, "MobileSettings.xml", sidecar(sidecars, "MobileSettings.xml").map_or(&[][..], |s| s.data.as_slice()))?;
    add(
        writer,
        &mut names,
        &mut report,
        "META-INF/metadata.xml",
        sidecar(sidecars, "META-INF/metadata.xml").map_or(DEFAULT_METADATA.as_bytes(), |s| s.data.as_slice()),
    )?;

    for (entry, xml, warnings) in plan.symbol_files() {
        for message in warnings {
            warn(&mut report, &entry, message);
        }
        add(writer, &mut names, &mut report, &entry, xml.as_bytes())?;
        report.symbols = report.symbols.saturating_add(1);
    }
    for blob in &plan.blobs {
        add(writer, &mut names, &mut report, &blob.entry, &blob.data)?;
        report.media_entries = report.media_entries.saturating_add(1);
    }
    for extra in sidecars {
        let lower = extra.name.to_ascii_lowercase();
        let handled = matches!(lower.as_str(), "mobilesettings.xml" | "meta-inf/metadata.xml") || Some(&extra.name) == marker.map(|m| &m.name);
        let cache = lower == "bin/symdepend.cache";
        if handled || (cache && !options.keep_caches) {
            continue;
        }
        add(writer, &mut names, &mut report, &extra.name, &extra.data)?;
    }
    Ok(report)
}

fn io_error(context: &Path, source: std::io::Error) -> XflError {
    XflError::Archive(ArchiveError::io(context.display().to_string(), source))
}

/// The path with `suffix` added to its file name (`a.fla` becomes `a.fla.tmp`).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

/// Writes a zipped `.fla` in memory.
///
/// # Errors
/// See [`write_to`].
pub fn write_fla_bytes(doc: &Document, store: &MediaStore, sidecars: &[Sidecar], options: &WriteOptions) -> Result<(Vec<u8>, WriteReport), XflError> {
    let mut zip = ZipWriter::with_mimetype(Vec::new(), XFL_MIMETYPE, anim_archive::WriteOptions::default())?;
    let mut report = write_to(&mut zip, doc, store, sidecars, options)?;
    report.entries = report.entries.saturating_add(1);
    report.bytes = report.bytes.saturating_add(u64::try_from(XFL_MIMETYPE.len()).unwrap_or(u64::MAX));
    Ok((ArchiveWriter::finish(zip)?, report))
}

/// Writes a zipped `.fla` file. The zip is written next to the target under a temporary name and renamed over it,
/// so a failure never leaves a half-written file at `path`.
///
/// # Errors
/// See [`write_to`]; also when the file cannot be created or renamed.
pub fn write_fla(path: impl AsRef<Path>, doc: &Document, store: &MediaStore, sidecars: &[Sidecar], options: &WriteOptions) -> Result<WriteReport, XflError> {
    let path = path.as_ref();
    let temporary = sibling(path, ".tmp");
    let result = write_fla_file(&temporary, doc, store, sidecars, options);
    match result {
        Ok(report) => {
            std::fs::rename(&temporary, path).map_err(|e| {
                let _ = std::fs::remove_file(&temporary);
                io_error(path, e)
            })?;
            Ok(report)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error)
        }
    }
}

fn write_fla_file(path: &Path, doc: &Document, store: &MediaStore, sidecars: &[Sidecar], options: &WriteOptions) -> Result<WriteReport, XflError> {
    let file = File::create(path).map_err(|e| io_error(path, e))?;
    let mut zip = ZipWriter::with_mimetype(BufWriter::new(file), XFL_MIMETYPE, anim_archive::WriteOptions::default())?;
    let mut report = write_to(&mut zip, doc, store, sidecars, options)?;
    report.entries = report.entries.saturating_add(1);
    report.bytes = report.bytes.saturating_add(u64::try_from(XFL_MIMETYPE.len()).unwrap_or(u64::MAX));
    let mut inner = zip.finish()?;
    inner.flush().map_err(|e| io_error(path, e))?;
    let file = inner.into_inner().map_err(|e| io_error(path, e.into_error()))?;
    file.sync_all().map_err(|e| io_error(path, e))?;
    Ok(report)
}

/// Writes an unzipped XFL folder: `dir` is created, and the marker file inside it is named after `options.name`.
///
/// # Errors
/// See [`write_to`].
pub fn write_xfl_folder(
    dir: impl AsRef<Path>,
    doc: &Document,
    store: &MediaStore,
    sidecars: &[Sidecar],
    options: &WriteOptions,
) -> Result<WriteReport, XflError> {
    let mut writer = DirectoryWriter::create(dir.as_ref())?;
    let report = write_to(&mut writer, doc, store, sidecars, options)?;
    ArchiveWriter::finish(writer)?;
    Ok(report)
}

/// Writes to `path`, choosing the form by its name: `*.fla` is a zip; `*.xfl` is the marker of a folder (written in
/// the file's parent folder, which is created); anything else is the folder itself.
///
/// # Errors
/// See [`write_to`].
pub fn write_path(path: impl AsRef<Path>, doc: &Document, store: &MediaStore, sidecars: &[Sidecar], options: &WriteOptions) -> Result<WriteReport, XflError> {
    let path = path.as_ref();
    let stem = path.file_stem().and_then(|s| s.to_str());
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let mut options = options.clone();
    match extension.as_deref() {
        Some("fla") => {
            if let Some(stem) = stem {
                stem.clone_into(&mut options.name);
            }
            write_fla(path, doc, store, sidecars, &options)
        }
        Some("xfl") => {
            if let Some(stem) = stem {
                stem.clone_into(&mut options.name);
            }
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
            write_xfl_folder(parent, doc, store, sidecars, &options)
        }
        _ => write_xfl_folder(path, doc, store, sidecars, &options),
    }
}

/// Writes what [`crate::read_with`] returned: the document, its media and its sidecars (see [`write_path`]).
///
/// # Errors
/// See [`write_to`].
pub fn write_loaded(path: impl AsRef<Path>, loaded: &Loaded) -> Result<WriteReport, XflError> {
    write_path(path, &loaded.document, &loaded.media, &loaded.sidecars, &WriteOptions::default())
}
