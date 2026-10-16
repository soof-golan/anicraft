//! Importing artwork into the library: the code behind File > Import and the `library.import` command.

use std::path::Path;

use anim_core::{Affine, num};
use anim_scene::{BitmapCompression, BitmapInstance, BitmapItem, Element, LibraryItem};

use crate::mcp::AniDoc;

/// Picks a library name that is free: `name`, then `name 2`, `name 3`, ...
fn free_name(doc: &anim_scene::Document, wanted: &str) -> String {
    if doc.library.find_index(wanted).is_none() {
        return wanted.to_owned();
    }
    let (stem, ext) = wanted.rsplit_once('.').map_or((wanted, ""), |(s, e)| (s, e));
    for n in 2..10_000 {
        let candidate = if ext.is_empty() { format!("{stem} {n}") } else { format!("{stem} {n}.{ext}") };
        if doc.library.find_index(&candidate).is_none() {
            return candidate;
        }
    }
    wanted.to_owned()
}

/// What an import produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// The library name of the new bitmap.
    pub name: String,
    /// Its size in pixels.
    pub size: [f64; 2],
}

/// Imports artwork into the library (and onto the stage when `to_stage`). Returns the library name.
///
/// Raster images are stored as they are. PSD, AI/PDF and SVG files are rasterised at their own size (their layers are
/// flattened); the editor keeps no vector import yet. `name` overrides the library name (a free variant is chosen when
/// it is taken).
///
/// # Errors
/// A message for unreadable or unsupported files.
pub fn import_artwork_as(doc: &mut AniDoc, path: &Path, name: Option<&str>, to_stage: bool) -> Result<Imported, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file_name = path.file_name().map_or_else(|| "image.png".to_owned(), |n| n.to_string_lossy().into_owned());
    let (data, default_name, compression, size) = if anim_artwork::detect(&bytes).is_some() {
        let tree = anim_artwork::import(&bytes, &anim_artwork::Limits::default()).map_err(|e| format!("{file_name}: {e}"))?;
        let mut store = anim_display::ImageStore::new();
        let list = anim_artwork::to_display(&tree, &mut store);
        let bounds = tree.root.bounds().unwrap_or_else(|| anim_core::Rect::new(0.0, 0.0, list.size.width.max(1.0), list.size.height.max(1.0)));
        let width = u32::try_from(num::ceil_f64_to_i32(bounds.width().max(1.0))).unwrap_or(1).clamp(1, 8192);
        let height = u32::try_from(num::ceil_f64_to_i32(bounds.height().max(1.0))).unwrap_or(1).clamp(1, 8192);
        let options = anim_render::RenderOptions::new(width, height)
            .with_view(Affine::translate((-bounds.x0, -bounds.y0)))
            .with_background(anim_render::Background::Transparent);
        let image = anim_render::render(&list, &store, &options).map_err(|e| e.to_string())?;
        let png = anim_export::encode_png(image.width, image.height, &anim_render::straight_rgba(&image)).map_err(|e| e.to_string())?;
        let stem = path.file_stem().map_or_else(|| "artwork".to_owned(), |s| s.to_string_lossy().into_owned());
        (png, format!("{stem}.png"), BitmapCompression::Lossless, [f64::from(image.width), f64::from(image.height)])
    } else {
        let image = anicraft_xfl::decode_bitmap_bytes(&bytes).map_err(|e| format!("{file_name}: {e}"))?;
        let jpeg = bytes.starts_with(&[0xFF, 0xD8]);
        (bytes, file_name.clone(), if jpeg { BitmapCompression::Photo } else { BitmapCompression::Lossless }, [f64::from(image.width), f64::from(image.height)])
    };
    let wanted = name.filter(|n| !n.is_empty()).unwrap_or(&default_name);
    let name = free_name(doc.document(), wanted);
    let media = doc.media.insert(data).map_err(|e| e.to_string())?;
    let item_id = doc.document().library.fresh_item_id(&name);
    let mut item = BitmapItem::new(&name, item_id);
    item.href = Some(file_name);
    item.media = Some(media);
    item.compression = compression;
    item.size = Some(size);
    doc.editor.document.library.push(LibraryItem::Bitmap(item)).map_err(|e| e.to_string())?;
    if to_stage {
        place_bitmap(doc, &name, size)?;
    }
    Ok(Imported { name, size })
}

/// [`import_artwork_as`] with the default library name; returns just the name.
///
/// # Errors
/// A message for unreadable or unsupported files.
pub fn import_artwork(doc: &mut AniDoc, path: &Path, to_stage: bool) -> Result<String, String> {
    import_artwork_as(doc, path, None, to_stage).map(|i| i.name)
}

/// Puts a bitmap instance centred on the stage on the current layer and frame.
///
/// # Errors
/// When the layer or frame does not exist.
pub fn place_bitmap(doc: &mut AniDoc, name: &str, size: [f64; 2]) -> Result<(), String> {
    let id = doc.editor.state.timeline.clone();
    let (layer, frame) = (doc.editor.state.layer, doc.editor.state.frame);
    let (w, h) = (doc.document().width, doc.document().height);
    let matrix = Affine::translate(((w - size[0]) / 2.0, (h - size[1]) / 2.0));
    let at = anim_scene::edit::add_element(&mut doc.editor.document, &id, layer, frame, Element::Bitmap(BitmapInstance::new(name, matrix))).map_err(|e| e.to_string())?;
    let keyframe = doc.document().timeline(&id).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).map_or(0, |k| k.index);
    doc.editor.state.selection = anicraft_tools::Selection::one(anicraft_tools::ElementRef::top_level(layer, keyframe, at));
    Ok(())
}
