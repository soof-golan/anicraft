//! `DOMDocument.xml`, the symbol files and the media entries, and the plan that names them.

use std::borrow::Cow;
use std::collections::HashSet;

use anim_archive::names::{escape_filename, href_to_entry_name, is_safe_relative, normalize_name};
use anim_core::Rgba8;
use anim_scene::{BitmapCompression, BitmapItem, Document, LibraryItem, MediaId, MediaStore, SoundItem, Symbol, VideoItem};
use rayon::prelude::{IntoParallelRefIterator, ParallelIterator};

use super::out::{Attrs, DOCUMENT_INDENT, XmlOut, fmt_rounded};
use super::scene::timeline;
use crate::media::{BitmapFormat, decode_bitmap_bytes, encode_lossless_bitmap, lossless_header, sniff_bitmap};

/// Symbol files rendered at once by [`Plan::symbol_files`].
const SYMBOL_BATCH: usize = 64;

/// One `bin/` entry to write.
#[derive(Debug)]
pub struct Blob<'a> {
    /// Entry name, `bin/M 1 1700000001.dat`.
    pub entry: String,
    pub data: Cow<'a, [u8]>,
}

/// What the writer decided for one media item.
#[derive(Debug, Default, Clone)]
struct MediaPlan {
    /// The `*DataHRef` to write.
    href: Option<String>,
    /// `originalCompressionType` to write for a bitmap (it changes when the bytes had to be converted).
    compression: Option<BitmapCompression>,
    /// Pixel size to write for a bitmap that had none.
    size: Option<[f64; 2]>,
}

/// The names and bytes of everything a document turns into.
pub struct Plan<'a> {
    doc: &'a Document,
    /// `Include href` of each symbol, in library order.
    symbol_hrefs: Vec<String>,
    /// Plan of each library item, by position in `doc.library.items` (symbols and folders get the default).
    media: Vec<MediaPlan>,
    pub blobs: Vec<Blob<'a>>,
    pub warnings: Vec<String>,
}

impl<'a> Plan<'a> {
    /// Names the symbol files and media entries and prepares the media bytes.
    #[must_use]
    pub fn new(doc: &'a Document, store: &'a MediaStore) -> Self {
        let mut plan = Self { doc, symbol_hrefs: Vec::new(), media: Vec::new(), blobs: Vec::new(), warnings: Vec::new() };
        plan.name_symbols();
        plan.prepare_media(store);
        plan
    }

    /// The symbols of the document in the order of `symbol_hrefs`.
    fn symbols(&self) -> impl Iterator<Item = &'a Symbol> + use<'a> {
        self.doc.library.symbols()
    }

    fn name_symbols(&mut self) {
        let mut used: HashSet<String> = HashSet::new();
        let mut hrefs = Vec::new();
        let doc = self.doc;
        for symbol in doc.library.symbols() {
            let wanted = symbol.href.as_deref().filter(|h| !h.is_empty() && is_safe_relative(&href_to_entry_name(h, "LIBRARY")));
            let mut href = wanted.map_or_else(|| default_symbol_href(&symbol.name), str::to_owned);
            let mut n = 1u32;
            while !used.insert(normalize_name(&href).to_lowercase()) {
                n = n.saturating_add(1);
                let stem = href.strip_suffix(".xml").unwrap_or(&href);
                let stem = stem.rsplit_once(" (").map_or(stem, |(s, _)| s);
                href = format!("{stem} ({n}).xml");
            }
            hrefs.push(href);
        }
        self.symbol_hrefs = hrefs;
    }

    fn prepare_media(&mut self, store: &'a MediaStore) {
        let mut used: HashSet<String> = HashSet::new();
        let mut taken_by: Vec<(String, MediaId)> = Vec::new();
        // Existing names first, so generated ones never take them.
        for item in &self.doc.library.items {
            let href = match item {
                LibraryItem::Bitmap(b) => b.bitmap_data_href.as_deref(),
                LibraryItem::Sound(s) => s.sound_data_href.as_deref(),
                LibraryItem::Video(v) => v.video_data_href.as_deref(),
                _ => None,
            };
            if let Some(href) = href {
                used.insert(normalize_name(href).to_lowercase());
            }
        }
        let mut counter = 0u32;
        let doc = self.doc;
        let mut plans = Vec::with_capacity(doc.library.items.len());
        for item in &doc.library.items {
            let mut plan = MediaPlan::default();
            match item {
                LibraryItem::Bitmap(b) => {
                    let blob = bitmap_bytes(b, store, &mut plan, &mut self.warnings);
                    self.place(b.bitmap_data_href.as_deref(), b.media, blob, &b.name, &mut plan, &mut used, &mut taken_by, &mut counter);
                }
                LibraryItem::Sound(s) => {
                    let blob = plain_bytes(s.media, store);
                    self.place(s.sound_data_href.as_deref(), s.media, blob, &s.name, &mut plan, &mut used, &mut taken_by, &mut counter);
                }
                LibraryItem::Video(v) => {
                    let blob = plain_bytes(v.media, store);
                    self.place(v.video_data_href.as_deref(), v.media, blob, &v.name, &mut plan, &mut used, &mut taken_by, &mut counter);
                }
                LibraryItem::Symbol(_) | LibraryItem::Font(_) | LibraryItem::Folder(_) => {}
            }
            plans.push(plan);
        }
        self.media = plans;
    }

    /// Decides the `href` of one media item and queues its bytes.
    #[allow(clippy::too_many_arguments, reason = "one call site per media kind; a struct would only rename the arguments")]
    fn place(
        &mut self,
        existing: Option<&str>,
        media: Option<MediaId>,
        blob: Option<Cow<'a, [u8]>>,
        item_name: &str,
        plan: &mut MediaPlan,
        used: &mut HashSet<String>,
        taken_by: &mut Vec<(String, MediaId)>,
        counter: &mut u32,
    ) {
        let Some(data) = blob else {
            // No bytes: keep the reference, nothing to write.
            if media.is_some() {
                self.warnings.push(format!("{item_name}: its media is not in the store; the data entry is left out"));
            }
            plan.href = existing.map(str::to_owned);
            return;
        };
        let content_key = media.unwrap_or(MediaId(0));
        if let Some(href) = existing.filter(|h| is_safe_relative(&href_to_entry_name(h, "bin"))) {
            let key = normalize_name(href).to_lowercase();
            // The same name for the same bytes is written once; a clash with different bytes gets a new name.
            match taken_by.iter().find(|(k, _)| *k == key) {
                Some((_, id)) if *id == content_key => {
                    plan.href = Some(href.to_owned());
                    return;
                }
                None => {
                    taken_by.push((key, content_key));
                    plan.href = Some(href.to_owned());
                    self.blobs.push(Blob { entry: href_to_entry_name(href, "bin"), data });
                    return;
                }
                Some(_) => self.warnings.push(format!("{item_name}: data entry name {href:?} is used by other media; a new name is generated")),
            }
        }
        let href = loop {
            *counter = counter.saturating_add(1);
            let stamp = 1_700_000_000u32.saturating_add(*counter);
            let candidate = format!("M {counter} {stamp}.dat");
            if used.insert(candidate.to_lowercase()) {
                break candidate;
            }
        };
        taken_by.push((normalize_name(&href).to_lowercase(), content_key));
        self.blobs.push(Blob { entry: href_to_entry_name(&href, "bin"), data });
        plan.href = Some(href);
    }

    fn media_plan(&self, index: usize) -> MediaPlan {
        self.media.get(index).cloned().unwrap_or_default()
    }

    /// `DOMDocument.xml`.
    #[must_use]
    pub fn document_xml(&self) -> (String, Vec<String>) {
        let doc = self.doc;
        let mut out = XmlOut::with_indent(DOCUMENT_INDENT);
        let current = u32::try_from(doc.current_timeline).unwrap_or(0).saturating_add(1);
        // Animate leaves out the stage settings of a new document (550 x 400 at 24 fps on white), and so do we.
        let attrs = Attrs::new()
            .f_unless("width", doc.width, 550.0)
            .f_unless("height", doc.height, 400.0)
            .f_unless("frameRate", doc.frame_rate, 24.0)
            .color_unless("backgroundColor", doc.background_color, Rgba8::WHITE)
            .f_unless("viewAngle3D", doc.perspective_angle, anim_core::projective::DEFAULT_PERSPECTIVE_ANGLE)
            .f_opt("vanishingPoint3DX", doc.vanishing_point.map(|p| p.x))
            .f_opt("vanishingPoint3DY", doc.vanishing_point.map(|p| p.y))
            .u("currentTimeline", current)
            .extra(&doc.extra);
        out.open_root("DOMDocument", &attrs);

        // Unknown library elements the reader parked in `extra` go back to the section they came from.
        let (folder_extra, rest): (Vec<&String>, Vec<&String>) = doc.extra.children.iter().partition(|c| root_tag(c) == Some("DOMFolderItem"));
        let (media_extra, rest): (Vec<&String>, Vec<&String>) =
            rest.into_iter().partition(|c| root_tag(c).is_some_and(|t| t.starts_with("DOM") && t.ends_with("Item")));

        if doc.library.folders().next().is_some() || !folder_extra.is_empty() {
            out.open("folders", &Attrs::new());
            for folder in doc.library.folders() {
                out.open(
                    "DOMFolderItem",
                    &Attrs::new().s("name", &folder.name).s("itemID", &folder.item_id).b_unless("isExpanded", folder.is_expanded, false).extra(&folder.extra),
                );
                for child in &folder.extra.children {
                    out.raw("DOMFolderItem", child);
                }
                out.close();
            }
            for raw in folder_extra {
                out.raw("folders", raw);
            }
            out.close();
        }

        let has_media =
            doc.library.items.iter().any(|i| matches!(i, LibraryItem::Bitmap(_) | LibraryItem::Sound(_) | LibraryItem::Video(_) | LibraryItem::Font(_)));
        if has_media || !media_extra.is_empty() {
            out.open("media", &Attrs::new());
            for (index, item) in doc.library.items.iter().enumerate() {
                let plan = self.media_plan(index);
                match item {
                    LibraryItem::Bitmap(b) => bitmap_item(&mut out, b, &plan),
                    LibraryItem::Sound(s) => sound_item(&mut out, s, &plan),
                    LibraryItem::Video(v) => video_item(&mut out, v, &plan),
                    LibraryItem::Font(f) => {
                        out.open("DOMFontItem", &Attrs::new().s("name", &f.name).s("itemID", &f.item_id).s_opt("font", f.font.as_deref()).extra(&f.extra));
                        for child in &f.extra.children {
                            out.raw("DOMFontItem", child);
                        }
                        out.close();
                    }
                    LibraryItem::Symbol(_) | LibraryItem::Folder(_) => {}
                }
            }
            for raw in media_extra {
                out.raw("media", raw);
            }
            out.close();
        }

        if !self.symbol_hrefs.is_empty() {
            out.open("symbols", &Attrs::new());
            for (symbol, href) in self.symbols().zip(&self.symbol_hrefs) {
                let mut attrs = Attrs::new().s("href", href);
                let mut id_written = false;
                for (name, value) in &symbol.include_attrs {
                    if name == "lastModified" && !id_written && !symbol.item_id.is_empty() {
                        attrs = attrs.s("itemID", &symbol.item_id);
                        id_written = true;
                    }
                    attrs = attrs.pair(name, value);
                }
                if !id_written && !symbol.item_id.is_empty() {
                    attrs = attrs.s("itemID", &symbol.item_id);
                }
                out.leaf("Include", &attrs);
            }
            out.close();
        }

        out.open("timelines", &Attrs::new());
        for scene in &doc.scenes {
            timeline(&mut out, scene);
        }
        out.close();

        for raw in rest {
            out.raw("DOMDocument", raw);
        }
        out.close();
        out.finish()
    }

    /// The symbol files: entry name (`LIBRARY/...`) and text, in library order.
    ///
    /// The texts are produced in parallel, [`SYMBOL_BATCH`] files at a time, so that memory stays bounded while the
    /// caller writes them out.
    pub fn symbol_files(&self) -> impl Iterator<Item = (String, String, Vec<String>)> + '_ {
        let pairs: Vec<(&Symbol, &String)> = self.symbols().zip(&self.symbol_hrefs).collect();
        (0..pairs.len()).step_by(SYMBOL_BATCH).flat_map(move |start| {
            let batch = pairs.get(start..start.saturating_add(SYMBOL_BATCH).min(pairs.len())).unwrap_or_default();
            let files: Vec<_> = batch
                .par_iter()
                .map(|&(symbol, href)| {
                    let (xml, warnings) = symbol_xml(symbol);
                    (href_to_entry_name(href, "LIBRARY"), xml, warnings)
                })
                .collect();
            files
        })
    }
}

/// The element name a kept XML fragment starts with.
fn root_tag(fragment: &str) -> Option<&str> {
    let rest = fragment.trim_start().strip_prefix('<')?;
    let end = rest.find(|c: char| c.is_whitespace() || c == '/' || c == '>')?;
    rest.get(..end)
}

fn default_symbol_href(name: &str) -> String {
    format!("{}.xml", escape_filename(name))
}

fn plain_bytes(media: Option<MediaId>, store: &MediaStore) -> Option<Cow<'_, [u8]>> {
    media.and_then(|id| store.get(id)).map(Cow::Borrowed)
}

/// The bytes a bitmap item is stored as. Lossless items must hold Animate's lossless format and photo items JPEG;
/// bytes in another image format are converted, and the plan records the compression that results.
fn bitmap_bytes<'a>(item: &BitmapItem, store: &'a MediaStore, plan: &mut MediaPlan, warnings: &mut Vec<String>) -> Option<Cow<'a, [u8]>> {
    let bytes = store.get(item.media?)?;
    let format = sniff_bitmap(bytes);
    if format == BitmapFormat::Lossless
        && let Ok(header) = lossless_header(bytes)
        && item.size.is_none()
    {
        plan.size = Some([f64::from(header.width), f64::from(header.height)]);
    }
    let keep = match (item.compression, format) {
        (BitmapCompression::Lossless, BitmapFormat::Lossless) | (BitmapCompression::Photo, BitmapFormat::Jpeg) | (_, BitmapFormat::Unknown) => true,
        (BitmapCompression::Photo | BitmapCompression::Lossless, _) => false,
    };
    if format == BitmapFormat::Unknown {
        warnings.push(format!("{}: bitmap data in an unknown format is written as it is", item.name));
    }
    if keep {
        return Some(Cow::Borrowed(bytes));
    }
    match decode_bitmap_bytes(bytes).and_then(|image| encode_lossless_bitmap(&image).map(|dat| (image.width, image.height, dat))) {
        Ok((width, height, dat)) => {
            plan.compression = Some(BitmapCompression::Lossless);
            if item.size.is_none() {
                plan.size = Some([f64::from(width), f64::from(height)]);
            }
            Some(Cow::Owned(dat))
        }
        Err(error) => {
            warnings.push(format!("{}: bitmap converted to the lossless format failed ({error}); written as it is", item.name));
            Some(Cow::Borrowed(bytes))
        }
    }
}

fn bitmap_item(out: &mut XmlOut, b: &BitmapItem, plan: &MediaPlan) {
    let compression = plan.compression.unwrap_or(b.compression);
    let size = b.size.or(plan.size);
    let attrs = Attrs::new()
        .s("name", &b.name)
        .s("itemID", &b.item_id)
        .extra(&b.extra)
        .b_unless("allowSmoothing", b.smoothing, false)
        .s("originalCompressionType", compression.xfl_name())
        .u_opt("quality", b.quality.map(u32::from))
        .s_opt("href", b.href.as_deref())
        .s_opt("bitmapDataHRef", plan.href.as_deref())
        .s_opt("frameRight", size.map(|s| fmt_rounded(s[0] * 20.0, 6)).as_deref())
        .s_opt("frameBottom", size.map(|s| fmt_rounded(s[1] * 20.0, 6)).as_deref());
    out.open("DOMBitmapItem", &attrs);
    for child in &b.extra.children {
        out.raw("DOMBitmapItem", child);
    }
    out.close();
}

fn sound_item(out: &mut XmlOut, s: &SoundItem, plan: &MediaPlan) {
    let attrs = Attrs::new()
        .s("name", &s.name)
        .s("itemID", &s.item_id)
        .extra(&s.extra)
        .s_opt("href", s.href.as_deref())
        .s_opt("soundDataHRef", plan.href.as_deref())
        .s_opt("format", s.format.as_deref())
        .u64_opt("sampleCount", s.sample_count);
    out.open("DOMSoundItem", &attrs);
    for child in &s.extra.children {
        out.raw("DOMSoundItem", child);
    }
    out.close();
}

fn video_item(out: &mut XmlOut, v: &VideoItem, plan: &MediaPlan) {
    let attrs = Attrs::new()
        .s("name", &v.name)
        .s("itemID", &v.item_id)
        .extra(&v.extra)
        .s_opt("videoDataHRef", plan.href.as_deref())
        .f_opt("fps", v.fps)
        .f_opt("width", v.width)
        .f_opt("height", v.height);
    out.open("DOMVideoItem", &attrs);
    for child in &v.extra.children {
        out.raw("DOMVideoItem", child);
    }
    out.close();
}

/// A symbol file (`LIBRARY/...`).
#[must_use]
pub fn symbol_xml(symbol: &Symbol) -> (String, Vec<String>) {
    let mut out = XmlOut::new();
    let mut attrs = Attrs::new()
        .s("name", &symbol.name)
        .s("itemID", &symbol.item_id)
        .s_unless("symbolType", symbol.kind.xfl_name(), "movie clip")
        .u64_opt("lastModified", symbol.last_modified);
    if let Some(link) = &symbol.linkage {
        let only_flag = link.export_in_first_frame.is_none()
            && link.identifier.is_none()
            && link.class_name.is_none()
            && link.base_class.is_none()
            && !link.import_for_runtime_sharing
            && link.url.is_none();
        // The reader makes a linkage out of any linkage attribute, so a lone `false` flag has to stay visible.
        if link.export_for_actionscript || only_flag {
            attrs = attrs.b("linkageExportForAS", link.export_for_actionscript);
        }
        attrs = attrs
            .b_opt("linkageExportInFirstFrame", link.export_in_first_frame)
            .s_opt("linkageIdentifier", link.identifier.as_deref())
            .s_opt("linkageClassName", link.class_name.as_deref())
            .s_opt("linkageBaseClass", link.base_class.as_deref())
            .b_unless("linkageImportForRS", link.import_for_runtime_sharing, false)
            .s_opt("linkageURL", link.url.as_deref());
    }
    if let Some(grid) = symbol.scale9_grid {
        attrs = attrs.f("scaleGridLeft", grid.x0).f("scaleGridTop", grid.y0).f("scaleGridRight", grid.x1).f("scaleGridBottom", grid.y1);
    }
    out.open_root("DOMSymbolItem", &attrs.extra(&symbol.extra));
    out.open("timeline", &Attrs::new());
    timeline(&mut out, &symbol.timeline);
    out.close();
    for child in &symbol.extra.children {
        out.raw("DOMSymbolItem", child);
    }
    out.close();
    out.finish()
}
