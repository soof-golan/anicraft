//! Renders one frame of a document's scene with `anim-render`, to check what the reader produced.
//!
//! This is a deliberately small flattener (the real one is `anim-timeline`): layers bottom to top, the keyframe
//! covering the frame, graphic symbols advanced by their loop mode, shapes through `EdgeShape::to_display`, colour
//! effects, blend modes and filters on instances. Text, masks and tweens are not evaluated.
//!
//! `cargo run -p anicraft-xfl --example render_frame -- FILE.fla OUT.png [FRAME] [SCENE]`

#![allow(clippy::arithmetic_side_effects, clippy::chunks_exact_to_as_chunks, clippy::too_many_lines, reason = "developer tool")]

use std::collections::HashMap;
use std::path::PathBuf;

use anicraft_xfl::{ReadOptions, decode_bitmap, read_with};
use anim_core::Size;
use anim_display::{Bitmap, DisplayList, Group, ImageId, ImageStore, Node};
use anim_render::{RenderOptions, render};
use anim_scene::{Document, Element, InstanceFilter, LayerKind, LoopMode, SymbolKind, Timeline};

struct Resolver {
    ids: HashMap<String, ImageId>,
}

impl anim_shape::ImageResolver for Resolver {
    fn resolve(&self, path: &str) -> Option<ImageId> {
        self.ids.get(path).copied()
    }
}

fn flatten(doc: &Document, timeline: &Timeline, frame: u32, resolver: &Resolver, depth: usize) -> Vec<Node> {
    let mut out = Vec::new();
    if depth > 16 {
        return out;
    }
    for layer in timeline.layers.iter().rev() {
        let kind = timeline.resolved_kind(timeline.layers.iter().position(|l| std::ptr::eq(l, layer)).unwrap_or(0)).unwrap_or(layer.kind);
        if !layer.visible || matches!(kind, LayerKind::Guide | LayerKind::Folder | LayerKind::Mask | LayerKind::Camera) {
            continue;
        }
        let Some(keyframe) = layer.keyframe_at(frame) else { continue };
        for element in &keyframe.elements {
            element_nodes(doc, element, frame.saturating_sub(keyframe.index), resolver, depth, &mut out);
        }
    }
    out
}

fn element_nodes(doc: &Document, element: &Element, since_key: u32, resolver: &Resolver, depth: usize, out: &mut Vec<Node>) {
    match element {
        Element::Shape(s) => {
            let children = s.shape.to_display(resolver);
            out.push(Node::Group(Group { transform: s.matrix, children, ..Group::default() }));
        }
        Element::Group(g) => {
            let mut children = Vec::new();
            for m in &g.members {
                element_nodes(doc, m, since_key, resolver, depth + 1, &mut children);
            }
            out.push(Node::Group(Group { transform: g.matrix, children, ..Group::default() }));
        }
        Element::Bitmap(b) => {
            if let Some(image) = resolver.ids.get(&b.library_item_name) {
                out.push(Node::Bitmap(Bitmap { transform: b.matrix, image: *image, smoothing: true }));
            }
        }
        Element::SymbolInstance(i) => {
            let Some(symbol) = doc.library.find_symbol(&i.library_item_name) else { return };
            let length = symbol.timeline.frame_count().max(1);
            let kind = i.effective_kind(&doc.library);
            let inner = if kind == SymbolKind::Graphic {
                match i.effective_loop_mode() {
                    LoopMode::SingleFrame => i.first_frame,
                    LoopMode::Loop => (i.first_frame + since_key) % length,
                    LoopMode::PlayOnce => (i.first_frame + since_key).min(length - 1),
                }
            } else {
                0
            };
            let children = flatten(doc, &symbol.timeline, inner, resolver, depth + 1);
            let filters = i.filters.iter().filter_map(InstanceFilter::to_display).cloned().collect();
            out.push(Node::Group(Group { transform: i.matrix, color: i.color, blend: i.blend_mode, filters, children, ..Group::default() }));
        }
        Element::Text(_) | Element::Unknown(_) => {}
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().ok_or("usage: render_frame FILE OUT.png [FRAME] [SCENE]")?);
    let out = PathBuf::from(args.next().ok_or("usage: render_frame FILE OUT.png [FRAME] [SCENE]")?);
    let frame: u32 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(0);
    let scene: usize = args.next().map(|s| s.parse()).transpose()?.unwrap_or(0);
    let loaded = read_with(&path, &ReadOptions::default())?;
    let doc = &loaded.document;
    let mut images = ImageStore::new();
    let mut resolver = Resolver { ids: HashMap::new() };
    for bitmap in doc.library.bitmaps() {
        if let Some(id) = bitmap.media
            && let Ok(image) = decode_bitmap(&loaded.media, id)
        {
            resolver.ids.insert(bitmap.name.clone(), images.insert(image));
        }
    }
    let timeline = doc.scenes.get(scene).ok_or("no such scene")?;
    let nodes = flatten(doc, timeline, frame, &resolver, 0);
    let list = DisplayList { size: Size::new(doc.width, doc.height), background: Some(doc.background_color), nodes };
    let options = RenderOptions::fit_stage(list.size, 960, 540);
    let image = render(&list, &images, &options)?;
    anim_testkit::write_png(&out, image.width, image.height, &anim_render::straight_rgba(&image))?;
    println!("wrote {} ({}x{}, scene {scene}, frame {frame})", out.display(), image.width, image.height);
    Ok(())
}
