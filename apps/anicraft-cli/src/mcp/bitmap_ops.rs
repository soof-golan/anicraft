//! Modify > Bitmap: Convert to Bitmap and Trace Bitmap.
//!
//! Convert to Bitmap renders the selected objects through the same evaluator and rasteriser as an export, crops the
//! result to the objects' extent and puts it in the library as a lossless bitmap with an instance in their place.
//! Trace Bitmap reads the bitmap's pixels and turns them into filled vector shapes with `vectorcraft-trace`.

use anicraft_tools::{ElementRef, Selection};
use anim_core::{Affine, Point, num};
use anim_scene::{BitmapCompression, BitmapInstance, BitmapItem, Document, Element, Frame, LibraryItem, ShapeElement, TimelineId, edit};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use anim_shape::{EdgeShape, FillStyle, PathInput};
use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_trace::{Method, Mode, Palette, Raster, TraceParams, trace_within};

use super::arrange_ops::{element_mut, failed, has_selection, one_parent, select_paths, siblings_mut};
use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

/// The largest rendering Convert to Bitmap makes, per side.
const MAX_SIDE: u32 = 8192;
/// The most anchors a trace may produce before it gives up.
const MAX_ANCHORS: usize = 400_000;

/// Registers the commands of this module.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(all()).expect("bitmap command ids are unique");
}

/// The commands.
#[must_use]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("modify.convert_to_bitmap", "Convert to Bitmap", convert_to_bitmap)
            .in_menu(&["Modify"])
            .enabled_when(has_selection)
            .describe("Renders the selected objects of one keyframe into a new library bitmap (named `name`, default \"Bitmap\") and replaces them by an instance of it. Filters, blend modes and colour effects are baked in.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"properties":{"name":{"type":"string","minLength":1}}}"#),
        CommandSpec::new("modify.trace_bitmap", "Trace Bitmap", trace_bitmap)
            .in_menu(&["Modify", "Bitmap"])
            .enabled_when(has_selection)
            .describe("Traces the selected bitmap instances into vector shapes. `color_threshold` 1..500 (default 100; a lower value keeps more colours), `minimum_area` 1..1000 pixels (default 8; smaller regions are dropped), `curve_fit` pixels, very_tight, tight, normal, smooth or very_smooth, `corner_threshold` many, normal or few. The bitmap is replaced by the shapes unless `keep_bitmap` is set.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"properties":{"color_threshold":{"type":"integer","minimum":1,"maximum":500},"minimum_area":{"type":"integer","minimum":1,"maximum":1000},"curve_fit":{"enum":["pixels","very_tight","tight","normal","smooth","very_smooth"]},"corner_threshold":{"enum":["many","normal","few"]},"keep_bitmap":{"type":"boolean","default":false}}}"#),
    ]
}

/// A rendering of some objects: PNG bytes, pixel size and the stage position of the top-left corner.
struct Rendering {
    png: Vec<u8>,
    size: [u32; 2],
    origin: Point,
}

fn rasterize(doc: &AniDoc, elements: &[Element]) -> Result<Rendering, CommandError> {
    let mut scratch = Document::new(doc.document().width, doc.document().height, doc.document().frame_rate);
    scratch.library = doc.document().library.clone();
    scratch.background_color = anim_core::Rgba8 { r: 0, g: 0, b: 0, a: 0 };
    let layer = scratch.scenes.first_mut().and_then(|s| s.layers.first_mut()).ok_or_else(|| CommandError::failed("no scratch layer"))?;
    layer.frames = vec![Frame::new(0, 1, elements.to_vec())];
    let mut ev = crate::project::evaluator_for(&scratch, &doc.media, false);
    let list = ev.frame(0, 0);
    let bounds = anim_render::content_bounds(&list, ev.images(), Affine::IDENTITY).ok_or_else(|| CommandError::failed("the selected objects draw nothing"))?;
    let x0 = bounds.x0.floor();
    let y0 = bounds.y0.floor();
    let side = |lo: f64, hi: f64| u32::try_from(num::ceil_f64_to_i32((hi - lo).max(1.0))).unwrap_or(1).clamp(1, MAX_SIDE);
    let (w, h) = (side(x0, bounds.x1), side(y0, bounds.y1));
    let options = anim_render::RenderOptions::new(w, h).with_view(Affine::translate((-x0, -y0))).with_background(anim_render::Background::Transparent);
    let image = anim_render::render(&list, ev.images(), &options).map_err(failed)?;
    let png = anim_export::encode_png(image.width, image.height, &anim_render::straight_rgba(&image)).map_err(failed)?;
    Ok(Rendering { png, size: [image.width, image.height], origin: Point::new(x0, y0) })
}

fn free_name(document: &Document, wanted: &str) -> String {
    if document.library.find_index(wanted).is_none() {
        return wanted.to_owned();
    }
    (2..10_000).map(|n| format!("{wanted} {n}")).find(|c| document.library.find_index(c).is_none()).unwrap_or_else(|| wanted.to_owned())
}

/// Adds a lossless bitmap to the library and returns its name.
fn add_bitmap(doc: &mut AniDoc, wanted: &str, rendering: &Rendering) -> Result<String, CommandError> {
    let name = free_name(doc.document(), wanted);
    let media = doc.media.insert(rendering.png.clone()).map_err(failed)?;
    let item_id = doc.document().library.fresh_item_id(&name);
    let mut item = BitmapItem::new(&name, item_id);
    item.href = Some(format!("{name}.png"));
    item.media = Some(media);
    item.compression = BitmapCompression::Lossless;
    item.size = Some([f64::from(rendering.size[0]), f64::from(rendering.size[1])]);
    doc.editor.document.library.push(LibraryItem::Bitmap(item)).map_err(failed)?;
    Ok(name)
}

/// Pastes `elements` (not in the document) as one bitmap on `layer` at `frame`; returns the index of the instance.
pub(super) fn paste_as_bitmap(doc: &mut AniDoc, id: &TimelineId, layer: usize, frame: u32, elements: &[Element]) -> Result<usize, CommandError> {
    let rendering = rasterize(doc, elements)?;
    let name = add_bitmap(doc, "Bitmap", &rendering)?;
    let instance = BitmapInstance::new(&name, Affine::translate(rendering.origin.to_vec2()));
    edit::add_element(doc.document_mut(), id, layer, frame, Element::Bitmap(instance)).map_err(failed)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConvertParams {
    name: Option<String>,
}

fn convert_to_bitmap(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ConvertParams = parse_params(params)?;
    let sel = one_parent(doc)?;
    let id = doc.editor.state.timeline.clone();
    let elements: Vec<Element> = {
        let siblings = siblings_mut(doc.document_mut(), &id, &sel).ok_or_else(|| CommandError::failed("the selection is gone"))?;
        sel.indices.iter().filter_map(|&i| siblings.get(i).cloned()).collect()
    };
    // Members of a group are drawn inside the group's matrix: render them as the group would.
    let rendering = rasterize(doc, &elements)?;
    let name = add_bitmap(doc, p.name.as_deref().unwrap_or("Bitmap"), &rendering)?;
    let siblings = siblings_mut(doc.document_mut(), &id, &sel).ok_or_else(|| CommandError::failed("the selection is gone"))?;
    for &i in sel.indices.iter().rev() {
        siblings.remove(i);
    }
    let at = sel.indices.first().copied().unwrap_or(0).min(siblings.len());
    siblings.insert(at, Element::Bitmap(BitmapInstance::new(&name, Affine::translate(rendering.origin.to_vec2()))));
    select_paths(doc, &sel, [at]);
    Ok(json!({ "bitmap": name, "width": rendering.size[0], "height": rendering.size[1] }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TraceParamsIn {
    color_threshold: Option<u32>,
    minimum_area: Option<u32>,
    curve_fit: Option<String>,
    corner_threshold: Option<String>,
    #[serde(default)]
    keep_bitmap: bool,
}

/// The tracer's settings for the Trace Bitmap dialog's choices.
fn trace_settings(p: &TraceParamsIn) -> Result<TraceParams, CommandError> {
    let threshold = p.color_threshold.unwrap_or(100).clamp(1, 500);
    let colors = 2000_u32.checked_div(threshold).unwrap_or(2).clamp(2, 64);
    let paths = match p.curve_fit.as_deref().unwrap_or("normal") {
        "pixels" => 100.0,
        "very_tight" => 90.0,
        "tight" => 75.0,
        "normal" => 55.0,
        "smooth" => 35.0,
        "very_smooth" => 15.0,
        other => return Err(CommandError::invalid_params(format!("unknown curve fit {other:?}"))),
    };
    let corners = match p.corner_threshold.as_deref().unwrap_or("normal") {
        "many" => 90.0,
        "normal" => 60.0,
        "few" => 30.0,
        other => return Err(CommandError::invalid_params(format!("unknown corner threshold {other:?}"))),
    };
    Ok(TraceParams {
        mode: Mode::Color,
        palette: Palette::Limited,
        colors,
        paths,
        corners,
        noise: p.minimum_area.unwrap_or(8).clamp(1, 1000),
        method: Method::Abutting,
        ..TraceParams::default()
    })
}

fn trace_bitmap(doc: &mut AniDoc, params: &Value) -> Out {
    let p: TraceParamsIn = parse_params(params)?;
    let settings = trace_settings(&p)?;
    let id = doc.editor.state.timeline.clone();
    let items = doc.editor.state.selection.items.clone();
    // Bitmaps first: they are found by name, and the replaced elements keep their place in the stack.
    let mut targets: Vec<(ElementRef, String, Affine)> = Vec::new();
    for r in &items {
        if let Some(Element::Bitmap(b)) = element_mut(doc.document_mut(), &id, r) {
            targets.push((r.clone(), b.library_item_name.clone(), b.matrix));
        }
    }
    if targets.is_empty() {
        return Err(CommandError::failed("Trace Bitmap needs a selected bitmap"));
    }
    let mut traced = Vec::new();
    let mut shapes = 0_usize;
    for (r, name, matrix) in &targets {
        let shape = trace_item(doc, name, &settings)?;
        shapes = shapes.saturating_add(shape.1);
        traced.push((r.clone(), *matrix, shape.0));
    }
    let mut new_items = Vec::new();
    for (r, matrix, shape) in traced {
        let element = Element::Shape(ShapeElement { shape, matrix, ..ShapeElement::default() });
        if p.keep_bitmap {
            let layer = r.layer;
            let at = edit::add_element(doc.document_mut(), &id, layer, r.keyframe, element).map_err(failed)?;
            new_items.push(ElementRef::top_level(layer, r.keyframe, at));
        } else if let Some(slot) = element_mut(doc.document_mut(), &id, &r) {
            *slot = element;
            new_items.push(r);
        }
    }
    doc.editor.state.selection = Selection { items: new_items, scope: None };
    Ok(json!({ "bitmaps": targets.len(), "regions": shapes }))
}

/// Traces the library bitmap `name`: the shape and how many coloured regions it has.
fn trace_item(doc: &AniDoc, name: &str, settings: &TraceParams) -> Result<(EdgeShape, usize), CommandError> {
    let item = doc
        .document()
        .library
        .items
        .iter()
        .find_map(|i| match i {
            LibraryItem::Bitmap(b) if b.name == name => Some(b),
            _ => None,
        })
        .ok_or_else(|| CommandError::failed(format!("no bitmap named {name:?} in the library")))?;
    let bytes = item.media.and_then(|m| doc.media.get(m)).ok_or_else(|| CommandError::failed(format!("bitmap {name:?} has no pixel data")))?;
    let image = anicraft_xfl::decode_bitmap_bytes(bytes).map_err(failed)?;
    let raster = Raster::new(image.width, image.height, anim_render::straight_rgba(&image));
    let result = trace_within(&raster, settings, MAX_ANCHORS).map_err(failed)?;
    let inputs: Vec<PathInput> = result
        .paths
        .iter()
        .map(|t| {
            let [r, g, b] = t.color;
            PathInput::new(t.path.to_bezpath(), Some(FillStyle::Solid(anim_core::Rgba8 { r, g, b, a: 255 })), None)
        })
        .collect();
    if inputs.is_empty() {
        return Err(CommandError::failed("the trace found nothing: lower the colour threshold or the minimum area"));
    }
    let count = inputs.len();
    Ok((EdgeShape::from_paths(&inputs), count))
}

#[cfg(test)]
#[allow(
    clippy::many_single_char_names,
    clippy::arithmetic_side_effects,
    clippy::manual_assert_eq,
    clippy::assert_is_empty,
    clippy::redundant_closure_for_method_calls,
    reason = "test code"
)]
mod tests {
    use anim_session::Session;

    use super::*;
    use crate::mcp::registry;

    fn session() -> Session<AniDoc> {
        Session::new(AniDoc::default(), registry())
    }

    #[test]
    fn converting_to_a_bitmap_puts_a_picture_in_the_library() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 20, "y": 30, "width": 60, "height": 40, "fill": "#ff0000", "object_drawing": true })).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        let report = s.execute("modify.convert_to_bitmap", &json!({})).unwrap();
        let (w, h) = (report["width"].as_u64().unwrap(), report["height"].as_u64().unwrap());
        assert!((60..=68).contains(&w) && (40..=48).contains(&h), "{w}x{h}");
        let frame = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().clone();
        assert_eq!(frame.elements.len(), 1);
        let Element::Bitmap(b) = &frame.elements[0] else { panic!("not a bitmap") };
        assert_eq!(b.library_item_name, "Bitmap");
        let c = b.matrix.as_coeffs();
        assert!((c[4] - 20.0).abs() < 4.0 && (c[5] - 30.0).abs() < 4.0, "{c:?}");
        assert!(s.doc().media.len() == 1);
        s.undo();
        assert!(s.doc().editor.document.library.find_index("Bitmap").is_none());
    }

    #[test]
    fn tracing_a_bitmap_gives_shapes_of_its_colours() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 40, "height": 40, "fill": "#000000", "object_drawing": true })).unwrap();
        s.execute("draw.rectangle", &json!({ "x": 40, "y": 0, "width": 40, "height": 40, "fill": "#ff0000", "object_drawing": true })).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        s.execute("modify.convert_to_bitmap", &json!({ "name": "pic" })).unwrap();
        let report = s.execute("modify.trace_bitmap", &json!({ "curve_fit": "tight" })).unwrap();
        assert!(report["regions"].as_u64().unwrap() >= 2, "{report}");
        let frame = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().clone();
        let Element::Shape(shape) = &frame.elements[0] else { panic!("not a shape") };
        assert!(!shape.is_drawing_object);
        let area = anicraft_tools::modify::fill_area(&shape.shape);
        assert!((area - 3200.0).abs() < 200.0, "{area}");
        assert!(s.execute("modify.trace_bitmap", &json!({})).is_err(), "no bitmap is selected any more");
        assert!(s.execute("modify.trace_bitmap", &json!({ "curve_fit": "wobbly" })).is_err());
    }

    #[test]
    fn convert_needs_something_that_draws() {
        let mut s = session();
        assert!(s.execute("modify.convert_to_bitmap", &json!({})).is_err());
    }
}
