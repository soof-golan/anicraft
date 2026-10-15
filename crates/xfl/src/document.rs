//! `DOMDocument.xml`: stage settings, the library sections and the scenes.

use anim_archive::Container;
use anim_core::Rgba8;
use anim_scene::{Document, Extra, Library, MediaStore};
use roxmltree::Node;

use crate::library::{Sources, read_folders, read_media, read_symbols};
use crate::timeline::read_timeline;
use crate::xml::{Attrs, Ctx, element_children};

/// Most scenes read.
const MAX_SCENES: usize = 10_000;

/// Reads a `<DOMDocument>` element, following symbol includes and loading media through `container`.
pub fn read_document(ctx: &mut Ctx<'_>, root: Node<'_, '_>, container: &Container, store: &mut MediaStore, load_media: bool) -> Document {
    let mut attrs = Attrs::new(root);
    let mut doc = Document::new(550.0, 400.0, 24.0);
    doc.scenes.clear();
    doc.width = attrs.f64(ctx, "width").unwrap_or(550.0);
    doc.height = attrs.f64(ctx, "height").unwrap_or(400.0);
    doc.frame_rate = attrs.f64(ctx, "frameRate").unwrap_or(24.0);
    doc.background_color = attrs.color(ctx, "backgroundColor").unwrap_or(Rgba8::WHITE);
    // XFL counts timelines from 1.
    let current = attrs.u32(ctx, "currentTimeline").unwrap_or(1);
    doc.current_timeline = usize::try_from(current.saturating_sub(1)).unwrap_or(0);
    let mut library = Library::new();
    let mut extra = Extra::new();
    let mut leftovers = Extra::new();
    let mut sources = Sources { container, store, load_media };
    for c in element_children(root) {
        match c.tag_name().name() {
            "folders" => read_folders(ctx, c, &mut library, &mut leftovers),
            "media" => read_media(ctx, c, &mut library, &mut leftovers, &mut sources),
            "symbols" => read_symbols(ctx, c, &mut library, &mut leftovers, &sources),
            "timelines" => {
                for t in element_children(c) {
                    if t.tag_name().name() != "DOMTimeline" {
                        ctx.keep_child(&mut extra, "timelines", t);
                    } else if doc.scenes.len() >= MAX_SCENES {
                        ctx.warn("too many scenes, the rest are dropped");
                        break;
                    } else {
                        doc.scenes.push(read_timeline(ctx, t));
                    }
                }
            }
            _ => ctx.keep_child(&mut extra, "DOMDocument", c),
        }
    }
    attrs.rest(ctx, &mut extra);
    extra.children.extend(leftovers.children);
    doc.extra = extra;
    doc.library = library;
    if doc.scenes.is_empty() {
        ctx.warn("document has no timeline; a blank scene is added");
        let blank = Document::new(doc.width, doc.height, doc.frame_rate);
        doc.scenes = blank.scenes;
    }
    if doc.current_timeline >= doc.scenes.len() {
        ctx.warn(format!("currentTimeline {current} is out of range; using the first scene"));
        doc.current_timeline = 0;
    }
    doc
}
