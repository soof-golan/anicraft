//! The Properties panel: what the selection (or the document) exposes for editing, and how an edit is applied.
//!
//! One description serves the panel and the `selection.set_property` command, so a person dragging a number and an
//! agent calling the tool take the same path, and both are one undo step.
//!
//! Property ids are flat strings. Object properties (`x`, `fill`, `name`, `size`, ...) apply to the single selected
//! object; `doc.*` to the document; `layer.*`, `frame.*` and `tween.*` to the layer and keyframe under the
//! playhead; `filter.N.*` to the N-th (0-based) filter of the selected instance; `bitmap.*` to the library item
//! behind the selected bitmap.
//!
//! * [`shape`]: fills and strokes of vector shapes and drawing objects;
//! * [`instance`]: symbol instances (type, looping, colour effect, blend, filters);
//! * [`text`]: character and paragraph styles of text fields;
//! * [`timeline`]: the layer, the keyframe and its tween.

mod instance;
mod shape;
mod text;
mod timeline;

use anicraft_cli::mcp::AniDoc;
use anicraft_tools::ElementRef;
use anim_core::{Affine, Rgba8};
use anim_scene::{BitmapCompression, Document, Element, LibraryItem, TimelineId};
use anim_ui::property::{PropValue, Property};
use serde_json::Value;

/// Greys a row out: it is shown for information only.
trait ReadOnly {
    fn read_only(self) -> Self;
}

impl ReadOnly for Property {
    fn read_only(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Mutable access to the element a reference points at.
pub use anicraft_cli::mcp::editing::element_mut;

const COMPRESSIONS: [&str; 2] = ["Lossless", "Photo (JPEG)"];

fn element<'a>(doc: &'a Document, id: &TimelineId, r: &ElementRef) -> Option<&'a Element> {
    anicraft_tools::resolve_ref(doc, id, r).map(|x| x.element)
}

fn linear(m: Affine) -> (f64, f64, f64) {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let rotation = b.atan2(a).to_degrees();
    let sx = a.hypot(b);
    let det = a.mul_add(d, -(b * c));
    let sy = c.hypot(d) * if det < 0.0 { -1.0 } else { 1.0 };
    (rotation, sx, sy)
}

/// A 0.0 ..= 1.0 fraction as a percentage.
fn pct(v: f32) -> f64 {
    f64::from(v) * 100.0
}

/// A percentage (clamped to `min..=max`) as a fraction.
fn fraction(percent: f64, min: f64, max: f64) -> f32 {
    anim_core::num::f64_to_f32(percent.clamp(min, max) / 100.0)
}

/// The range, precision and unit of a number row.
#[derive(Clone, Copy)]
struct Unit {
    min: f64,
    max: f64,
    decimals: usize,
    suffix: &'static str,
}

impl Unit {
    const fn new(min: f64, max: f64, decimals: usize, suffix: &'static str) -> Self {
        Self { min, max, decimals, suffix }
    }
}

/// A draggable number row in `section`.
fn number_row(id: &str, label: &str, value: f64, unit: Unit, section: &str) -> Property {
    let speed = if unit.decimals == 0 { 1.0 } else { 0.5 };
    Property::new(id, label, PropValue::number_in(value, unit.min, unit.max).precision(unit.decimals, speed).with_suffix(unit.suffix)).in_section(section)
}

/// A percentage row for a 0..=1 value.
fn percent_row(id: &str, label: &str, value: f32, section: &str) -> Property {
    Property::new(id, label, PropValue::number_in(pct(value), 0.0, 100.0).precision(0, 1.0).with_suffix(" %")).in_section(section)
}

fn number(value: &PropValue) -> Option<f64> {
    match value {
        PropValue::Number { value, .. } if value.is_finite() => Some(*value),
        _ => None,
    }
}

fn text(value: &PropValue) -> Option<String> {
    match value {
        PropValue::Text(t) => Some(t.clone()),
        _ => None,
    }
}

fn boolean(value: &PropValue) -> Option<bool> {
    match value {
        PropValue::Bool(b) => Some(*b),
        _ => None,
    }
}

fn colour(value: &PropValue) -> Option<Rgba8> {
    match value {
        PropValue::Color(c) => Some(*c),
        _ => None,
    }
}

fn choice(value: &PropValue) -> Option<usize> {
    match value {
        PropValue::Choice { selected, .. } => Some(*selected),
        _ => None,
    }
}

/// The message for a value of the wrong kind.
fn wrong(id: &str) -> String {
    format!("property {id:?} cannot take that value")
}

/// The properties to show for the current selection, or for the document and the playhead's layer and keyframe when
/// nothing is selected.
#[must_use]
pub fn describe(doc: &AniDoc) -> Vec<Property> {
    let editor = &doc.editor;
    let document = &editor.document;
    let selection = &editor.state.selection.items;
    let mut out = Vec::new();
    match selection.as_slice() {
        [] => {
            document_rows(document, &mut out);
            timeline::describe(document, &editor.state.timeline, editor.state.layer, editor.state.frame, &mut out);
        }
        [one] => describe_element(document, &editor.state.timeline, one, &mut out),
        many => {
            out.push(Property::new("multi", "Selected", PropValue::Text(format!("{} objects", many.len()))).in_section("Selection"));
            if let Some(bounds) =
                anicraft_tools::hit::transform_rect(Affine::IDENTITY, union_bounds(document, &editor.state.timeline, many).unwrap_or(anim_core::Rect::ZERO))
            {
                out.push(Property::new("size", "Size", PropValue::Text(format!("{:.0} x {:.0}", bounds.width(), bounds.height()))).in_section("Selection"));
            }
        }
    }
    out
}

/// Every property the document currently offers, whatever is selected: the panel's rows plus the document, the layer
/// and the keyframe. `selection.set_property` resolves ids against this.
#[must_use]
pub fn catalog(doc: &AniDoc) -> Vec<Property> {
    let mut out = describe(doc);
    let editor = &doc.editor;
    if !editor.state.selection.items.is_empty() {
        document_rows(&editor.document, &mut out);
        timeline::describe(&editor.document, &editor.state.timeline, editor.state.layer, editor.state.frame, &mut out);
    }
    out
}

fn document_rows(document: &Document, out: &mut Vec<Property>) {
    let size = |id: &str, label: &str, v: f64| {
        Property::new(id, label, PropValue::number_in(v, 1.0, 16000.0).precision(0, 1.0).with_suffix(" px")).in_section("Document")
    };
    out.push(size("doc.width", "Width", document.width));
    out.push(size("doc.height", "Height", document.height));
    out.push(
        Property::new("doc.fps", "Frame rate", PropValue::number_in(document.frame_rate, 1.0, 240.0).precision(2, 0.5).with_suffix(" fps"))
            .in_section("Document"),
    );
    out.push(Property::new("doc.background", "Stage colour", PropValue::Color(document.background_color)).in_section("Document"));
    let frames: u32 = document.scenes.iter().map(anim_scene::Timeline::frame_count).sum();
    out.push(
        Property::new("doc.scenes", "Scenes", PropValue::Text(format!("{} scene(s), {frames} frames", document.scenes.len())))
            .in_section("Document")
            .read_only(),
    );
    let symbols = document.library.symbols().count();
    out.push(
        Property::new("doc.library", "Library", PropValue::Text(format!("{} item(s), {symbols} symbol(s)", document.library.items.len())))
            .in_section("Document")
            .read_only(),
    );
}

fn union_bounds(doc: &Document, id: &TimelineId, refs: &[ElementRef]) -> Option<anim_core::Rect> {
    crate::view::selection_rects(doc, id, refs).into_iter().reduce(|a, b| a.union(b))
}

fn describe_element(doc: &Document, id: &TimelineId, r: &ElementRef, out: &mut Vec<Property>) {
    let Some(el) = element(doc, id, r) else { return };
    out.push(Property::new("title", "Type", PropValue::Text(crate::view::element_title(el))).in_section("Selection").read_only());
    let bounds = crate::view::selection_rects(doc, id, std::slice::from_ref(r)).into_iter().next();
    let origin = el.matrix().map_or((0.0, 0.0), |m| (m.translation().x, m.translation().y));
    let (x, y) = if matches!(el, Element::Shape(_)) { bounds.map_or(origin, |b| (b.x0, b.y0)) } else { origin };
    out.push(Property::new("x", "X", PropValue::number(x).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"));
    out.push(Property::new("y", "Y", PropValue::number(y).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"));
    if let Some(b) = bounds {
        out.push(
            Property::new("w", "Width", PropValue::number_in(b.width(), 0.0, 16000.0).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"),
        );
        out.push(
            Property::new("h", "Height", PropValue::number_in(b.height(), 0.0, 16000.0).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"),
        );
    }
    if !matches!(el, Element::Shape(_))
        && let Some(m) = el.matrix()
    {
        let (rotation, _, _) = linear(m);
        out.push(
            Property::new("rotation", "Rotation", PropValue::number_in(rotation, -360.0, 360.0).precision(1, 0.5).with_suffix(" deg"))
                .in_section("Position and size"),
        );
    }
    match el {
        Element::SymbolInstance(i) => instance::describe(i, &doc.library, out),
        Element::Text(t) => text::describe(t, out),
        Element::Shape(s) => shape::describe(s, out),
        Element::Bitmap(b) => {
            let names: Vec<String> = doc.library.bitmaps().map(|item| anicraft_xfl::display_name(&item.name).into_owned()).collect();
            let current = doc.library.bitmaps().position(|item| item.name == b.library_item_name).unwrap_or(0);
            let options: Vec<&str> = names.iter().map(String::as_str).collect();
            out.push(Property::new("source", "Source", PropValue::choice(current, &options)).in_section("Bitmap"));
            if let Some(item) = doc.library.find_bitmap(&b.library_item_name) {
                if let Some([w, h]) = item.size {
                    out.push(Property::new("bitmap.size", "Original size", PropValue::Text(format!("{w:.0} x {h:.0} px"))).in_section("Bitmap").read_only());
                }
                out.push(Property::new("bitmap.smoothing", "Allow smoothing", PropValue::Bool(item.smoothing)).in_section("Bitmap"));
                let photo = matches!(item.compression, BitmapCompression::Photo);
                out.push(Property::new("bitmap.compression", "Compression", PropValue::choice(usize::from(photo), &COMPRESSIONS)).in_section("Bitmap"));
                if photo {
                    out.push(
                        Property::new("bitmap.quality", "Quality", PropValue::number_in(f64::from(item.quality.unwrap_or(80)), 0.0, 100.0).precision(0, 1.0))
                            .in_section("Bitmap"),
                    );
                }
            }
        }
        Element::Group(g) => {
            out.push(Property::new("members", "Objects", PropValue::Text(format!("{} grouped", g.members.len()))).in_section("Group").read_only());
        }
        Element::Unknown(raw) => {
            out.push(Property::new("kept", "Kept as", PropValue::Text(raw.tag.clone())).in_section("Selection").read_only());
        }
    }
}

/// Applies an edit of the panel to the document.
///
/// # Errors
/// A message when the property does not exist for the selection or the value has the wrong kind or range.
pub fn apply(doc: &mut AniDoc, id: &str, value: &PropValue) -> Result<(), String> {
    if let Some(rest) = id.strip_prefix("doc.") {
        let d = &mut doc.editor.document;
        match rest {
            "width" => d.width = number(value).filter(|v| *v >= 1.0).ok_or_else(|| wrong(id))?,
            "height" => d.height = number(value).filter(|v| *v >= 1.0).ok_or_else(|| wrong(id))?,
            "fps" => d.frame_rate = number(value).filter(|v| *v >= 1.0 && *v <= 1000.0).ok_or_else(|| wrong(id))?,
            "background" => d.background_color = colour(value).ok_or_else(|| wrong(id))?,
            _ => return Err(format!("no document property {rest:?}")),
        }
        return Ok(());
    }
    if id.starts_with("layer.") || id.starts_with("frame.") || id.starts_with("tween.") {
        let timeline_id = doc.editor.state.timeline.clone();
        let (layer, frame) = (doc.editor.state.layer, doc.editor.state.frame);
        return timeline::apply(&mut doc.editor.document, &timeline_id, layer, frame, id, value);
    }
    let timeline = doc.editor.state.timeline.clone();
    let [target] = doc.editor.state.selection.items.as_slice() else { return Err("select exactly one object to edit its properties".to_owned()) };
    let target = target.clone();
    let stage_bounds = crate::view::selection_rects(&doc.editor.document, &timeline, std::slice::from_ref(&target)).into_iter().next();
    if let Some(field) = id.strip_prefix("bitmap.") {
        return apply_bitmap_item(&mut doc.editor.document, &timeline, &target, field, value);
    }
    if id == "symbol" || id == "source" {
        return swap_source(&mut doc.editor.document, &timeline, &target, id, value);
    }
    let el = element_mut(&mut doc.editor.document, &timeline, &target).ok_or("the selected object no longer exists")?;
    if matches!(id, "x" | "y" | "w" | "h" | "rotation") {
        return apply_transform(el, stage_bounds, id, value);
    }
    match el {
        Element::SymbolInstance(i) => instance::apply(i, id, value),
        Element::Text(t) => text::apply(t, id, value),
        Element::Shape(s) => shape::apply(s, id, value),
        _ => Err(format!("this object has no property {id:?}")),
    }
}

fn apply_transform(el: &mut Element, stage_bounds: Option<anim_core::Rect>, id: &str, value: &PropValue) -> Result<(), String> {
    let v = number(value).ok_or_else(|| wrong(id))?;
    let m = el.matrix().ok_or("this object cannot be transformed")?;
    let is_shape = matches!(el, Element::Shape(_));
    let new = match id {
        "x" | "y" => {
            let origin = (m.translation().x, m.translation().y);
            let current = if is_shape { stage_bounds.map_or(origin, |b| (b.x0, b.y0)) } else { origin };
            let (dx, dy) = if id == "x" { (v - current.0, 0.0) } else { (0.0, v - current.1) };
            Affine::translate((dx, dy)) * m
        }
        "w" | "h" => {
            let b = stage_bounds.ok_or("this object has no size")?;
            let (cur, anchor) = if id == "w" { (b.width(), b.x0) } else { (b.height(), b.y0) };
            if cur <= 0.0 || v <= 0.0 {
                return Err("width and height must be positive".to_owned());
            }
            let k = v / cur;
            let about = if id == "w" {
                Affine::translate((anchor, 0.0)) * Affine::scale_non_uniform(k, 1.0) * Affine::translate((-anchor, 0.0))
            } else {
                Affine::translate((0.0, anchor)) * Affine::scale_non_uniform(1.0, k) * Affine::translate((0.0, -anchor))
            };
            about * m
        }
        _ => {
            let (_, sx, sy) = linear(m);
            Affine::translate(m.translation()) * Affine::rotate(v.to_radians()) * Affine::scale_non_uniform(sx, sy)
        }
    };
    el.set_matrix(new);
    Ok(())
}

/// Swaps the library item an instance points at: another symbol for a symbol instance, another bitmap for a bitmap.
fn swap_source(doc: &mut Document, timeline: &TimelineId, target: &ElementRef, id: &str, value: &PropValue) -> Result<(), String> {
    let index = choice(value).ok_or_else(|| wrong(id))?;
    let picked =
        if id == "symbol" { doc.library.symbols().nth(index).map(|s| s.name.clone()) } else { doc.library.bitmaps().nth(index).map(|b| b.name.clone()) };
    let picked = picked.ok_or_else(|| format!("the library has no {id} number {index}"))?;
    if id == "symbol"
        && let TimelineId::Symbol(container) = timeline
        && doc.symbol_reaches(&picked, container)
    {
        return Err(format!("{picked:?} contains the symbol being edited; swapping it in would make a symbol contain itself"));
    }
    let el = element_mut(doc, timeline, target).ok_or("the selected object no longer exists")?;
    match el {
        Element::SymbolInstance(i) if id == "symbol" => {
            i.library_item_name = picked;
            // The instance keeps its own type only when it set one; a swapped-in graphic or clip decides otherwise.
            i.symbol_type = None;
            Ok(())
        }
        Element::Bitmap(b) if id == "source" => {
            b.library_item_name = picked;
            Ok(())
        }
        _ => Err(format!("this object has no property {id:?}")),
    }
}

/// Edits of the library bitmap behind the selected bitmap instance.
fn apply_bitmap_item(doc: &mut Document, timeline: &TimelineId, target: &ElementRef, field: &str, value: &PropValue) -> Result<(), String> {
    let name = match element(doc, timeline, target) {
        Some(Element::Bitmap(b)) => b.library_item_name.clone(),
        _ => return Err("select a bitmap to edit its library settings".to_owned()),
    };
    let Some(LibraryItem::Bitmap(item)) = doc.library.find_mut(&name) else { return Err(format!("the library has no bitmap {name:?}")) };
    match field {
        "smoothing" => item.smoothing = boolean(value).ok_or_else(|| wrong("bitmap.smoothing"))?,
        "compression" => {
            item.compression =
                if choice(value).ok_or_else(|| wrong("bitmap.compression"))? == 0 { BitmapCompression::Lossless } else { BitmapCompression::Photo };
        }
        "quality" => {
            let q = number(value).filter(|q| (0.0..=100.0).contains(q)).ok_or_else(|| wrong("bitmap.quality"))?;
            item.quality = Some(u8::try_from(anim_core::num::round_f64_to_u32(q)).unwrap_or(100));
        }
        _ => return Err(format!("bitmaps have no property {field:?}")),
    }
    Ok(())
}

/// Turns a JSON value into the kind of value the property currently holds.
///
/// # Errors
/// A message when the property is unknown or the JSON has the wrong shape.
pub fn from_json(doc: &AniDoc, id: &str, value: &Value) -> Result<PropValue, String> {
    let props = catalog(doc);
    let current = props
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no property {id:?} for the current selection; run the `properties` query to see what is there"))?;
    Ok(match &current.value {
        PropValue::Number { min, max, speed, decimals, suffix, .. } => PropValue::Number {
            value: value.as_f64().ok_or("expected a number")?,
            min: *min,
            max: *max,
            speed: *speed,
            decimals: *decimals,
            suffix: suffix.clone(),
        },
        PropValue::Bool(_) => PropValue::Bool(value.as_bool().ok_or("expected true or false")?),
        PropValue::Text(_) => PropValue::Text(value.as_str().ok_or("expected a string")?.to_owned()),
        PropValue::Color(_) => {
            let s = value.as_str().ok_or("expected a colour like \"#ff8800\" or \"#ff880080\"")?;
            PropValue::Color(Rgba8::from_hex(s).map_err(|e| format!("not a colour: {e:?}"))?)
        }
        PropValue::Choice { options, .. } => {
            let selected = match value {
                Value::String(s) => options.iter().position(|o| o.eq_ignore_ascii_case(s)).ok_or_else(|| format!("expected one of {options:?}"))?,
                Value::Number(n) => usize::try_from(n.as_u64().ok_or("expected an index")?).map_err(|e| e.to_string())?,
                _ => return Err(format!("expected one of {options:?}")),
            };
            PropValue::Choice { selected, options: options.clone() }
        }
    })
}

#[cfg(test)]
mod tests;
