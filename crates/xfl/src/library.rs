//! The `<folders>`, `<media>` and `<symbols>` sections of `DOMDocument.xml` and the symbol files they include.

use anim_archive::Container;
use anim_core::Rect;
use anim_scene::{
    BitmapCompression, BitmapItem, Extra, FolderItem, FontItem, Library, LibraryItem, Linkage, MediaId, MediaStore, SoundItem, Symbol, SymbolKind, VideoItem,
};
use roxmltree::Node;

use crate::timeline::read_timeline;
use crate::xml::{Attrs, Ctx, Sink, child, element_children};

/// Most `<Include>` elements followed.
const MAX_SYMBOLS: usize = 100_000;

/// What the section readers need besides the XML.
pub struct Sources<'a> {
    pub container: &'a Container,
    pub store: &'a mut MediaStore,
    pub load_media: bool,
}

/// Reads the `<folders>` section into `library`.
pub fn read_folders(ctx: &mut Ctx<'_>, node: Node<'_, '_>, library: &mut Library, leftovers: &mut Extra) {
    for c in element_children(node) {
        if c.tag_name().name() == "DOMFolderItem" {
            let mut attrs = Attrs::new(c);
            let name = attrs.string("name").unwrap_or_default();
            let item_id = attrs.string("itemID").unwrap_or_default();
            let is_expanded = attrs.bool(ctx, "isExpanded").unwrap_or(false);
            let mut extra = Extra::new();
            attrs.rest(ctx, &mut extra);
            for inner in element_children(c) {
                ctx.keep_child(&mut extra, "DOMFolderItem", inner);
            }
            push_item(ctx, library, LibraryItem::Folder(FolderItem { name, item_id, is_expanded, extra }));
        } else {
            ctx.keep_child(leftovers, "folders", c);
        }
    }
}

/// Reads the `<media>` section into `library`, loading the `bin/*.dat` blobs into the store.
pub fn read_media(ctx: &mut Ctx<'_>, node: Node<'_, '_>, library: &mut Library, leftovers: &mut Extra, sources: &mut Sources<'_>) {
    for c in element_children(node) {
        let item = match c.tag_name().name() {
            "DOMBitmapItem" => Some(LibraryItem::Bitmap(read_bitmap_item(ctx, c, sources))),
            "DOMSoundItem" => Some(LibraryItem::Sound(read_sound_item(ctx, c, sources))),
            "DOMVideoItem" => Some(LibraryItem::Video(read_video_item(ctx, c, sources))),
            "DOMFontItem" => Some(LibraryItem::Font(read_font_item(ctx, c))),
            _ => {
                ctx.keep_child(leftovers, "media", c);
                None
            }
        };
        if let Some(item) = item {
            push_item(ctx, library, item);
        }
    }
}

fn push_item(ctx: &mut Ctx<'_>, library: &mut Library, item: LibraryItem) {
    if library.find_index(item.name()).is_some() {
        ctx.warn(format!("duplicate library item name {:?} ({}); the later one is dropped", item.name(), item.kind_name()));
        return;
    }
    library.items.push(item);
}

/// Loads the blob an item points at; `None` (with a warning) when the entry is missing.
fn load_blob(ctx: &mut Ctx<'_>, sources: &mut Sources<'_>, href: &str, item: &str) -> Option<MediaId> {
    if !sources.load_media {
        return None;
    }
    let Some(entry) = sources.container.find_href(href, "bin") else {
        ctx.warn(format!("{item}: media entry {href:?} not found"));
        return None;
    };
    let bytes = match sources.container.read_entry(entry) {
        Ok(bytes) => bytes,
        Err(error) => {
            ctx.warn(format!("{item}: cannot read media {href:?}: {error}"));
            return None;
        }
    };
    match sources.store.insert(bytes) {
        Ok(id) => Some(id),
        Err(error) => {
            ctx.warn(format!("{item}: media {href:?} not loaded: {error}"));
            None
        }
    }
}

fn finish_item(ctx: &mut Ctx<'_>, node: Node<'_, '_>, attrs: Attrs<'_, '_>, extra: &mut Extra) {
    attrs.rest(ctx, extra);
    for inner in element_children(node) {
        ctx.keep_child(extra, node.tag_name().name(), inner);
    }
}

fn read_bitmap_item(ctx: &mut Ctx<'_>, node: Node<'_, '_>, sources: &mut Sources<'_>) -> BitmapItem {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let item_id = attrs.string("itemID").unwrap_or_default();
    let mut item = BitmapItem::new(name, item_id);
    item.href = attrs.string("href");
    item.bitmap_data_href = attrs.string("bitmapDataHRef");
    item.quality = attrs.u32(ctx, "quality").map(|q| u8::try_from(q.min(100)).unwrap_or(100));
    item.compression = attrs.take_with(ctx, "originalCompressionType", BitmapCompression::from_xfl_name).unwrap_or_default();
    item.smoothing = attrs.bool(ctx, "allowSmoothing").unwrap_or(false);
    let right = attrs.f64(ctx, "frameRight");
    let bottom = attrs.f64(ctx, "frameBottom");
    item.size = right.zip(bottom).map(|(w, h)| [w / 20.0, h / 20.0]);
    if let Some(href) = item.bitmap_data_href.clone() {
        item.media = load_blob(ctx, sources, &href, &item.name);
    }
    finish_item(ctx, node, attrs, &mut item.extra);
    item
}

fn read_sound_item(ctx: &mut Ctx<'_>, node: Node<'_, '_>, sources: &mut Sources<'_>) -> SoundItem {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let item_id = attrs.string("itemID").unwrap_or_default();
    let mut item = SoundItem::new(name, item_id);
    item.href = attrs.string("href");
    item.sound_data_href = attrs.string("soundDataHRef");
    item.format = attrs.string("format");
    item.sample_count = attrs.u64(ctx, "sampleCount");
    if let Some(href) = item.sound_data_href.clone() {
        item.media = load_blob(ctx, sources, &href, &item.name);
    }
    finish_item(ctx, node, attrs, &mut item.extra);
    item
}

fn read_video_item(ctx: &mut Ctx<'_>, node: Node<'_, '_>, sources: &mut Sources<'_>) -> VideoItem {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let item_id = attrs.string("itemID").unwrap_or_default();
    let video_data_href = attrs.string("videoDataHRef");
    let fps = attrs.f64(ctx, "fps");
    let width = attrs.f64(ctx, "width");
    let height = attrs.f64(ctx, "height");
    let media = video_data_href.clone().and_then(|href| load_blob(ctx, sources, &href, &name));
    let mut extra = Extra::new();
    finish_item(ctx, node, attrs, &mut extra);
    VideoItem { name, item_id, video_data_href, media, fps, width, height, extra }
}

fn read_font_item(ctx: &mut Ctx<'_>, node: Node<'_, '_>) -> FontItem {
    let mut attrs = Attrs::new(node);
    let name = attrs.string("name").unwrap_or_default();
    let item_id = attrs.string("itemID").unwrap_or_default();
    let font = attrs.string("font");
    let mut extra = Extra::new();
    finish_item(ctx, node, attrs, &mut extra);
    FontItem { name, item_id, font, extra }
}

/// Follows the `<symbols>` section: every `<Include href>` names a symbol file under `LIBRARY/`.
pub fn read_symbols(ctx: &mut Ctx<'_>, node: Node<'_, '_>, library: &mut Library, leftovers: &mut Extra, sources: &Sources<'_>) {
    let mut count = 0usize;
    for c in element_children(node) {
        if c.tag_name().name() != "Include" {
            ctx.keep_child(leftovers, "symbols", c);
            continue;
        }
        count = count.saturating_add(1);
        if count > MAX_SYMBOLS {
            ctx.warn("too many symbols, the rest are not read");
            break;
        }
        let href = c.attribute("href").unwrap_or_default().to_owned();
        let item_id = c.attribute("itemID").map(str::to_owned);
        // Every other attribute of the element is kept on the symbol as is.
        let include_attrs: Vec<(String, String)> =
            c.attributes().filter(|a| a.name() != "href" && a.name() != "itemID").map(|a| (a.name().to_owned(), a.value().to_owned())).collect();
        if let Some(symbol) = read_symbol_file(ctx.sink, sources.container, &href, item_id, include_attrs) {
            push_item(ctx, library, LibraryItem::Symbol(Box::new(symbol)));
        }
    }
}

fn read_symbol_file(
    sink: &mut Sink,
    container: &Container,
    href: &str,
    include_item_id: Option<String>,
    include_attrs: Vec<(String, String)>,
) -> Option<Symbol> {
    let fallback_name = href.strip_suffix(".xml").unwrap_or(href).to_owned();
    let Some(entry) = container.find_href(href, "LIBRARY") else {
        sink.warn("DOMDocument.xml", format!("symbol file {href:?} not found"));
        return None;
    };
    let file = entry.name.clone();
    let text = match container.read_text(&file) {
        Ok(text) => text,
        Err(error) => {
            sink.warn(&file, format!("cannot read: {error}"));
            return None;
        }
    };
    let xml = match crate::xml::parse(&text) {
        Ok(xml) => xml,
        Err(failure) => {
            sink.warn(&file, failure.to_string());
            return None;
        }
    };
    let root = xml.root_element();
    let mut ctx = Ctx::new(&text, &file, sink);
    if root.tag_name().name() != "DOMSymbolItem" {
        ctx.warn(format!("expected DOMSymbolItem, found {}", root.tag_name().name()));
        return None;
    }
    let mut attrs = Attrs::new(root);
    let name = attrs.string("name").unwrap_or(fallback_name);
    let item_id = attrs.string("itemID").or(include_item_id).unwrap_or_default();
    let mut symbol = Symbol::new(name, SymbolKind::MovieClip, item_id);
    symbol.kind = attrs.take_with(&mut ctx, "symbolType", SymbolKind::from_xfl_name).unwrap_or(SymbolKind::MovieClip);
    symbol.last_modified = attrs.u64(&mut ctx, "lastModified");
    symbol.href = Some(href.to_owned());
    symbol.include_attrs = include_attrs;
    symbol.linkage = read_linkage(&mut ctx, &mut attrs);
    symbol.scale9_grid = read_scale_grid(&mut ctx, root, &mut attrs);
    let mut found_timeline = false;
    for c in element_children(root) {
        if c.tag_name().name() == "timeline"
            && !found_timeline
            && let Some(t) = child(c, "DOMTimeline")
        {
            symbol.timeline = read_timeline(&mut ctx, t);
            found_timeline = true;
            continue;
        }
        ctx.keep_child(&mut symbol.extra, "DOMSymbolItem", c);
    }
    if !found_timeline {
        ctx.warn("symbol without a timeline");
    }
    attrs.rest(&mut ctx, &mut symbol.extra);
    Some(symbol)
}

fn read_linkage(ctx: &mut Ctx<'_>, attrs: &mut Attrs<'_, '_>) -> Option<Linkage> {
    let export_for_actionscript = attrs.bool(ctx, "linkageExportForAS");
    let export_in_first_frame = attrs.bool(ctx, "linkageExportInFirstFrame");
    let identifier = attrs.string("linkageIdentifier");
    let class_name = attrs.string("linkageClassName");
    let base_class = attrs.string("linkageBaseClass");
    let import_for_runtime_sharing = attrs.bool(ctx, "linkageImportForRS");
    let url = attrs.string("linkageURL");
    let any = export_for_actionscript.is_some()
        || export_in_first_frame.is_some()
        || identifier.is_some()
        || class_name.is_some()
        || base_class.is_some()
        || import_for_runtime_sharing.is_some()
        || url.is_some();
    any.then(|| Linkage {
        export_for_actionscript: export_for_actionscript.unwrap_or(false),
        export_in_first_frame,
        identifier,
        class_name,
        base_class,
        import_for_runtime_sharing: import_for_runtime_sharing.unwrap_or(false),
        url,
    })
}

fn read_scale_grid(ctx: &mut Ctx<'_>, root: Node<'_, '_>, attrs: &mut Attrs<'_, '_>) -> Option<Rect> {
    const KEYS: [&str; 4] = ["scaleGridLeft", "scaleGridTop", "scaleGridRight", "scaleGridBottom"];
    if !KEYS.iter().all(|k| root.attribute(*k).is_some()) {
        return None;
    }
    let left = attrs.f64(ctx, KEYS[0])?;
    let top = attrs.f64(ctx, KEYS[1])?;
    let right = attrs.f64(ctx, KEYS[2])?;
    let bottom = attrs.f64(ctx, KEYS[3])?;
    Some(Rect::new(left, top, right, bottom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_media_and_duplicates() {
        let text = r#"<media><DOMBitmapItem name="B" itemID="1" bitmapDataHRef="M 1.dat" frameRight="400" frameBottom="200" allowSmoothing="true" originalCompressionType="photo" quality="80" sourceLastImported="5"/>
        <DOMSoundItem name="S" itemID="2" format="44kHz 16bit Stereo" sampleCount="10" dataLength="40"/>
        <DOMSoundItem name="S" itemID="3"/>
        <DOMCompiledClipItem name="C"/></media>"#;
        let doc = roxmltree::Document::parse(text).unwrap();
        let mut sink = Sink::default();
        let mut writer = anim_archive::ZipWriter::in_memory();
        anim_archive::ArchiveWriter::add_file(&mut writer, "DOMDocument.xml", b"<DOMDocument/>").unwrap();
        let container = Container::from_bytes(anim_archive::ArchiveWriter::finish(writer).unwrap()).unwrap();
        let mut store = MediaStore::new();
        let mut library = Library::new();
        let mut leftovers = Extra::new();
        {
            let mut ctx = Ctx::new(text, "DOMDocument.xml", &mut sink);
            let mut sources = Sources { container: &container, store: &mut store, load_media: false };
            read_media(&mut ctx, doc.root_element(), &mut library, &mut leftovers, &mut sources);
        }
        assert_eq!(library.items.len(), 2);
        let bitmap = library.find_bitmap("B").unwrap();
        assert_eq!(bitmap.size, Some([20.0, 10.0]));
        assert_eq!(bitmap.compression, BitmapCompression::Photo);
        assert_eq!(bitmap.quality, Some(80));
        assert!(bitmap.smoothing);
        assert_eq!(bitmap.extra.attr("sourceLastImported"), Some("5"));
        assert_eq!(library.find_sound("S").unwrap().sample_count, Some(10));
        assert_eq!(leftovers.children.len(), 1);
        assert!(sink.warnings.iter().any(|w| w.message.contains("duplicate")));
    }
}
