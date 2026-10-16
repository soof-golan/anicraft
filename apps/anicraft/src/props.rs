//! The Properties panel: what the selection (or the document) exposes for editing, and how an edit is applied.
//!
//! One description serves the panel and the `selection.set_property` command, so a person dragging a number and an
//! agent calling the tool take the same path.

use anicraft_cli::mcp::AniDoc;
use anicraft_tools::{ElementRef, LibraryBounds};
use anim_core::{Affine, BlendMode, Rgba8, num};
use anim_scene::{ColorEffect, Document, Element, LoopMode, TextAlign, TextDirection, TimelineId};
use anim_shape::{FillStyle, StrokeStyle};
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

const BLENDS: [(&str, BlendMode); 12] = [
    ("Normal", BlendMode::Normal),
    ("Layer", BlendMode::Layer),
    ("Multiply", BlendMode::Multiply),
    ("Screen", BlendMode::Screen),
    ("Overlay", BlendMode::Overlay),
    ("Darken", BlendMode::Darken),
    ("Lighten", BlendMode::Lighten),
    ("Hard Light", BlendMode::HardLight),
    ("Difference", BlendMode::Difference),
    ("Add", BlendMode::Add),
    ("Subtract", BlendMode::Subtract),
    ("Invert", BlendMode::Invert),
];
const EFFECTS: [&str; 5] = ["None", "Brightness", "Tint", "Alpha", "Advanced"];
const LOOPS: [&str; 3] = ["Loop", "Play Once", "Single Frame"];
const DIRECTIONS: [&str; 3] = ["Auto", "Left to right", "Right to left"];
const ALIGNS: [&str; 4] = ["Left", "Right", "Center", "Justify"];

fn element<'a>(doc: &'a Document, id: &TimelineId, r: &ElementRef) -> Option<&'a Element> {
    anicraft_tools::resolve_ref(doc, id, r).map(|x| x.element)
}

/// Mutable access to the element a reference points at.
pub fn element_mut<'a>(doc: &'a mut Document, id: &TimelineId, r: &ElementRef) -> Option<&'a mut Element> {
    let layer = doc.timeline_mut(id)?.layers.get_mut(r.layer)?;
    let frame = layer.frames.iter_mut().find(|f| f.index == r.keyframe)?;
    let mut elements = &mut frame.elements;
    let (last, ancestors) = r.path.split_last()?;
    for &i in ancestors {
        let Some(Element::Group(g)) = elements.get_mut(i) else { return None };
        elements = &mut g.members;
    }
    elements.get_mut(*last)
}

fn linear(m: Affine) -> (f64, f64, f64) {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let rotation = b.atan2(a).to_degrees();
    let sx = a.hypot(b);
    let det = a.mul_add(d, -(b * c));
    let sy = c.hypot(d) * if det < 0.0 { -1.0 } else { 1.0 };
    (rotation, sx, sy)
}

fn colour_choice(effect: ColorEffect) -> usize {
    match effect {
        ColorEffect::None => 0,
        ColorEffect::Brightness(_) => 1,
        ColorEffect::Tint { .. } => 2,
        ColorEffect::Alpha(_) => 3,
        ColorEffect::Advanced(_) => 4,
    }
}

fn pct(v: f32) -> f64 {
    f64::from(v) * 100.0
}

/// The properties to show for the current selection, or for the document when nothing is selected.
#[must_use]
pub fn describe(doc: &AniDoc) -> Vec<Property> {
    let editor = &doc.editor;
    let document = &editor.document;
    let selection = &editor.state.selection.items;
    let mut out = Vec::new();
    match selection.as_slice() {
        [] => {
            out.push(Property::new("doc.width", "Width", PropValue::number_in(document.width, 1.0, 16000.0).precision(0, 1.0).with_suffix(" px")).in_section("Document"));
            out.push(Property::new("doc.height", "Height", PropValue::number_in(document.height, 1.0, 16000.0).precision(0, 1.0).with_suffix(" px")).in_section("Document"));
            out.push(Property::new("doc.fps", "Frame rate", PropValue::number_in(document.frame_rate, 1.0, 240.0).precision(2, 0.5).with_suffix(" fps")).in_section("Document"));
            out.push(Property::new("doc.background", "Stage colour", PropValue::Color(document.background_color)).in_section("Document"));
        }
        [one] => describe_element(document, &editor.state.timeline, one, &mut out),
        many => {
            out.push(Property::new("multi", "Selected", PropValue::Text(format!("{} objects", many.len()))).in_section("Selection"));
            if let Some(bounds) = anicraft_tools::hit::transform_rect(Affine::IDENTITY, union_bounds(document, &editor.state.timeline, many).unwrap_or(anim_core::Rect::ZERO)) {
                out.push(Property::new("size", "Size", PropValue::Text(format!("{:.0} x {:.0}", bounds.width(), bounds.height()))).in_section("Selection"));
            }
        }
    }
    out
}

fn union_bounds(doc: &Document, id: &TimelineId, refs: &[ElementRef]) -> Option<anim_core::Rect> {
    crate::view::selection_rects(doc, id, refs).into_iter().reduce(|a, b| a.union(b))
}

fn describe_element(doc: &Document, id: &TimelineId, r: &ElementRef, out: &mut Vec<Property>) {
    let Some(el) = element(doc, id, r) else { return };
    out.push(Property::new("title", "Type", PropValue::Text(crate::view::element_title(el))).in_section("Selection"));
    let bounds = crate::view::selection_rects(doc, id, std::slice::from_ref(r)).into_iter().next();
    let origin = el.matrix().map_or((0.0, 0.0), |m| (m.translation().x, m.translation().y));
    let (x, y) = if matches!(el, Element::Shape(_)) { bounds.map_or(origin, |b| (b.x0, b.y0)) } else { origin };
    out.push(Property::new("x", "X", PropValue::number(x).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"));
    out.push(Property::new("y", "Y", PropValue::number(y).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"));
    if let Some(b) = bounds {
        out.push(Property::new("w", "Width", PropValue::number_in(b.width(), 0.0, 16000.0).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"));
        out.push(Property::new("h", "Height", PropValue::number_in(b.height(), 0.0, 16000.0).precision(1, 0.5).with_suffix(" px")).in_section("Position and size"));
    }
    if !matches!(el, Element::Shape(_)) && let Some(m) = el.matrix() {
        let (rotation, _, _) = linear(m);
        out.push(Property::new("rotation", "Rotation", PropValue::number_in(rotation, -360.0, 360.0).precision(1, 0.5).with_suffix(" deg")).in_section("Position and size"));
    }
    match el {
        Element::SymbolInstance(i) => {
            out.push(Property::new("name", "Instance name", PropValue::Text(i.name.clone().unwrap_or_default())).in_section("Instance"));
            out.push(Property::new("visible", "Visible", PropValue::Bool(i.visible)).in_section("Instance"));
            if matches!(i.effective_kind(&doc.library), anim_scene::SymbolKind::Graphic) {
                let mode = match i.effective_loop_mode() {
                    LoopMode::Loop => 0,
                    LoopMode::PlayOnce => 1,
                    LoopMode::SingleFrame => 2,
                };
                out.push(Property::new("loop", "Looping", PropValue::choice(mode, &LOOPS)).in_section("Looping"));
                out.push(Property::new("first_frame", "First frame", PropValue::number_in(f64::from(i.first_frame) + 1.0, 1.0, 100_000.0).precision(0, 1.0)).in_section("Looping"));
            }
            out.push(Property::new("effect", "Style", PropValue::choice(colour_choice(i.color_effect), &EFFECTS)).in_section("Color effect"));
            match i.color_effect {
                ColorEffect::Brightness(b) => out.push(Property::new("brightness", "Brightness", PropValue::number_in(pct(b), -100.0, 100.0).precision(0, 1.0).with_suffix(" %")).in_section("Color effect")),
                ColorEffect::Tint { color, amount } => {
                    out.push(Property::new("tint", "Tint colour", PropValue::Color(color)).in_section("Color effect"));
                    out.push(Property::new("tint_amount", "Tint", PropValue::number_in(pct(amount), 0.0, 100.0).precision(0, 1.0).with_suffix(" %")).in_section("Color effect"));
                }
                ColorEffect::Alpha(a) => out.push(Property::new("alpha", "Alpha", PropValue::number_in(pct(a), 0.0, 100.0).precision(0, 1.0).with_suffix(" %")).in_section("Color effect")),
                ColorEffect::None | ColorEffect::Advanced(_) => {}
            }
            let blend = BLENDS.iter().position(|(_, b)| *b == i.blend_mode).unwrap_or(0);
            let names: Vec<&str> = BLENDS.iter().map(|(n, _)| *n).collect();
            out.push(Property::new("blend", "Blending", PropValue::choice(blend, &names)).in_section("Display"));
            out.push(Property::new("filters", "Filters", PropValue::Text(format!("{} applied", i.filters.len()))).in_section("Display").read_only());
        }
        Element::Text(t) => {
            let run = t.text_runs.first();
            out.push(Property::new("text", "Text", PropValue::Text(t.plain_text())).in_section("Character"));
            if let Some(run) = run {
                out.push(Property::new("font", "Family", PropValue::Text(run.face.clone())).in_section("Character"));
                out.push(Property::new("size", "Size", PropValue::number_in(f64::from(run.size), 1.0, 1000.0).precision(1, 0.5).with_suffix(" pt")).in_section("Character"));
                out.push(Property::new("text_color", "Colour", PropValue::Color(run.fill_color)).in_section("Character"));
                out.push(Property::new("bold", "Bold", PropValue::Bool(run.bold)).in_section("Character"));
                out.push(Property::new("italic", "Italic", PropValue::Bool(run.italic)).in_section("Character"));
                let dir = match run.direction {
                    TextDirection::Auto => 0,
                    TextDirection::LeftToRight => 1,
                    TextDirection::RightToLeft => 2,
                };
                out.push(Property::new("direction", "Direction", PropValue::choice(dir, &DIRECTIONS)).in_section("Paragraph"));
                let align = match run.alignment {
                    TextAlign::Left => 0,
                    TextAlign::Right => 1,
                    TextAlign::Center => 2,
                    TextAlign::Justify => 3,
                };
                out.push(Property::new("align", "Align", PropValue::choice(align, &ALIGNS)).in_section("Paragraph"));
            }
        }
        Element::Shape(s) => {
            if let Some(FillStyle::Solid(c)) = s.shape.fills.first() {
                out.push(Property::new("fill", "Fill", PropValue::Color(*c)).in_section("Fill and stroke"));
            }
            if let Some(stroke) = s.shape.strokes.first() {
                if let FillStyle::Solid(c) = &stroke.solid().fill {
                    out.push(Property::new("stroke", "Stroke", PropValue::Color(*c)).in_section("Fill and stroke"));
                }
                out.push(Property::new("stroke_width", "Stroke width", PropValue::number_in(stroke.weight(), 0.0, 200.0).precision(2, 0.1).with_suffix(" px")).in_section("Fill and stroke"));
            }
        }
        Element::Bitmap(b) => {
            out.push(Property::new("source", "Source", PropValue::Text(b.library_item_name.clone())).in_section("Instance").read_only());
        }
        Element::Group(_) | Element::Unknown(_) => {}
    }
}

fn number(value: &PropValue) -> Option<f64> {
    match value {
        PropValue::Number { value, .. } => Some(*value),
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

/// Applies an edit of the panel to the document.
///
/// # Errors
/// A message when the property does not exist for the selection or the value has the wrong kind or range.
pub fn apply(doc: &mut AniDoc, id: &str, value: &PropValue) -> Result<(), String> {
    let wrong = || format!("property {id:?} cannot take that value");
    if let Some(rest) = id.strip_prefix("doc.") {
        let d = &mut doc.editor.document;
        match rest {
            "width" => d.width = number(value).filter(|v| *v >= 1.0).ok_or_else(wrong)?,
            "height" => d.height = number(value).filter(|v| *v >= 1.0).ok_or_else(wrong)?,
            "fps" => d.frame_rate = number(value).filter(|v| *v >= 1.0 && *v <= 1000.0).ok_or_else(wrong)?,
            "background" => d.background_color = colour(value).ok_or_else(wrong)?,
            _ => return Err(format!("no document property {rest:?}")),
        }
        return Ok(());
    }
    let timeline = doc.editor.state.timeline.clone();
    let [target] = doc.editor.state.selection.items.as_slice() else { return Err("select exactly one object to edit its properties".to_owned()) };
    let target = target.clone();
    let stage_bounds = crate::view::selection_rects(&doc.editor.document, &timeline, std::slice::from_ref(&target)).into_iter().next();
    let library_bounds = LibraryBounds::new(&doc.editor.document.library);
    let _ = &library_bounds;
    let el = element_mut(&mut doc.editor.document, &timeline, &target).ok_or("the selected object no longer exists")?;
    match id {
        "x" | "y" | "w" | "h" | "rotation" => {
            let v = number(value).ok_or_else(wrong)?;
            if !v.is_finite() {
                return Err(wrong());
            }
            let m = el.matrix().ok_or("this object cannot be transformed")?;
            let is_shape = matches!(el, Element::Shape(_));
            let new = match id {
                "x" | "y" => {
                    let current = if is_shape { stage_bounds.map_or((m.translation().x, m.translation().y), |b| (b.x0, b.y0)) } else { (m.translation().x, m.translation().y) };
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
                    let about = if id == "w" { Affine::translate((anchor, 0.0)) * Affine::scale_non_uniform(k, 1.0) * Affine::translate((-anchor, 0.0)) } else { Affine::translate((0.0, anchor)) * Affine::scale_non_uniform(1.0, k) * Affine::translate((0.0, -anchor)) };
                    about * m
                }
                _ => {
                    let (rotation, sx, sy) = linear(m);
                    let t = m.translation();
                    let _ = rotation;
                    Affine::translate(t) * Affine::rotate(v.to_radians()) * Affine::scale_non_uniform(sx, sy)
                }
            };
            el.set_matrix(new);
            return Ok(());
        }
        _ => {}
    }
    match el {
        Element::SymbolInstance(i) => match id {
            "name" => {
                let t = text(value).ok_or_else(wrong)?;
                i.name = if t.is_empty() { None } else { Some(t) };
            }
            "visible" => i.visible = boolean(value).ok_or_else(wrong)?,
            "loop" => {
                i.loop_mode = Some(match choice(value).ok_or_else(wrong)? {
                    0 => LoopMode::Loop,
                    1 => LoopMode::PlayOnce,
                    _ => LoopMode::SingleFrame,
                });
            }
            "first_frame" => i.first_frame = u32::try_from(num::round_f64_to_i32(number(value).ok_or_else(wrong)?).max(1).saturating_sub(1)).unwrap_or(0),
            "effect" => {
                let effect = match choice(value).ok_or_else(wrong)? {
                    0 => ColorEffect::None,
                    1 => ColorEffect::Brightness(0.0),
                    2 => ColorEffect::Tint { color: Rgba8 { r: 255, g: 0, b: 0, a: 255 }, amount: 0.5 },
                    3 => ColorEffect::Alpha(1.0),
                    _ => ColorEffect::Advanced(i.color),
                };
                i.set_color_effect(effect);
            }
            "brightness" => i.set_color_effect(ColorEffect::Brightness(num::f64_to_f32(number(value).ok_or_else(wrong)?.clamp(-100.0, 100.0) / 100.0))),
            "alpha" => i.set_color_effect(ColorEffect::Alpha(num::f64_to_f32(number(value).ok_or_else(wrong)?.clamp(0.0, 100.0) / 100.0))),
            "tint" => {
                let ColorEffect::Tint { amount, .. } = i.color_effect else { return Err("set the style to Tint first".to_owned()) };
                i.set_color_effect(ColorEffect::Tint { color: colour(value).ok_or_else(wrong)?, amount });
            }
            "tint_amount" => {
                let ColorEffect::Tint { color, .. } = i.color_effect else { return Err("set the style to Tint first".to_owned()) };
                i.set_color_effect(ColorEffect::Tint { color, amount: num::f64_to_f32(number(value).ok_or_else(wrong)?.clamp(0.0, 100.0) / 100.0) });
            }
            "blend" => i.blend_mode = BLENDS.get(choice(value).ok_or_else(wrong)?).map(|(_, b)| *b).ok_or_else(wrong)?,
            _ => return Err(format!("instances have no property {id:?}")),
        },
        Element::Text(t) => {
            if id == "text" {
                let new = text(value).ok_or_else(wrong)?;
                let mut base = t.text_runs.first().cloned().ok_or("the text has no style to keep")?;
                new.clone_into(&mut base.characters);
                t.text_runs = vec![base];
                return Ok(());
            }
            for run in &mut t.text_runs {
                match id {
                    "font" => run.face = text(value).ok_or_else(wrong)?,
                    "size" => run.size = num::f64_to_f32(number(value).filter(|v| *v >= 1.0).ok_or_else(wrong)?),
                    "text_color" => run.fill_color = colour(value).ok_or_else(wrong)?,
                    "bold" => run.bold = boolean(value).ok_or_else(wrong)?,
                    "italic" => run.italic = boolean(value).ok_or_else(wrong)?,
                    "direction" => {
                        run.direction = match choice(value).ok_or_else(wrong)? {
                            1 => TextDirection::LeftToRight,
                            2 => TextDirection::RightToLeft,
                            _ => TextDirection::Auto,
                        };
                    }
                    "align" => {
                        run.alignment = match choice(value).ok_or_else(wrong)? {
                            1 => TextAlign::Right,
                            2 => TextAlign::Center,
                            3 => TextAlign::Justify,
                            _ => TextAlign::Left,
                        };
                    }
                    _ => return Err(format!("text has no property {id:?}")),
                }
            }
        }
        Element::Shape(s) => match id {
            "fill" => {
                let c = colour(value).ok_or_else(wrong)?;
                for fill in &mut s.shape.fills {
                    *fill = FillStyle::Solid(c);
                }
            }
            "stroke" => {
                let c = colour(value).ok_or_else(wrong)?;
                for stroke in &mut s.shape.strokes {
                    stroke.solid_mut().fill = FillStyle::Solid(c);
                }
            }
            "stroke_width" => {
                let w = number(value).filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(wrong)?;
                for stroke in &mut s.shape.strokes {
                    if let StrokeStyle::Solid(solid) = stroke {
                        solid.weight = w;
                    } else {
                        stroke.solid_mut().weight = w;
                    }
                }
            }
            _ => return Err(format!("shapes have no property {id:?}")),
        },
        _ => return Err(format!("this object has no property {id:?}")),
    }
    Ok(())
}

/// Turns a JSON value into the kind of value the property currently holds.
///
/// # Errors
/// A message when the property is unknown or the JSON has the wrong shape.
pub fn from_json(doc: &AniDoc, id: &str, value: &Value) -> Result<PropValue, String> {
    let props = describe(doc);
    let current = props.iter().find(|p| p.id == id).ok_or_else(|| format!("no property {id:?} for the current selection; call stage_elements or tools_state to see it"))?;
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
            let s = value.as_str().ok_or("expected a colour like \"#ff8800\"")?;
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
mod tests {
    use super::*;
    use anicraft_tools::Selection;
    use anim_scene::SymbolInstance;
    use serde_json::json;

    fn doc_with_instance() -> AniDoc {
        let mut doc = AniDoc::default();
        doc.editor.document.add_symbol("box", anim_scene::SymbolKind::Graphic).unwrap();
        let id = TimelineId::Scene(0);
        anim_scene::edit::add_symbol_instance(&mut doc.editor.document, &id, 0, 0, "box", Affine::translate((10.0, 20.0))).unwrap();
        doc.editor.state.selection = Selection::one(ElementRef::top_level(0, 0, 0));
        doc
    }

    #[test]
    fn nothing_selected_edits_the_document() {
        let mut doc = AniDoc::default();
        let props = describe(&doc);
        assert!(props.iter().any(|p| p.id == "doc.width"));
        apply(&mut doc, "doc.width", &PropValue::number(800.0)).unwrap();
        assert!((doc.editor.document.width - 800.0).abs() < f64::EPSILON);
        assert!(apply(&mut doc, "doc.fps", &PropValue::number(-1.0)).is_err());
    }

    #[test]
    fn instance_properties_round_trip() {
        let mut doc = doc_with_instance();
        let props = describe(&doc);
        let x = props.iter().find(|p| p.id == "x").unwrap();
        assert!(matches!(x.value, PropValue::Number { value, .. } if (value - 10.0).abs() < 1e-9));
        apply(&mut doc, "x", &PropValue::number(50.0)).unwrap();
        apply(&mut doc, "rotation", &PropValue::number(90.0)).unwrap();
        apply(&mut doc, "name", &PropValue::Text("hero".into())).unwrap();
        apply(&mut doc, "effect", &PropValue::choice(3, &EFFECTS)).unwrap();
        apply(&mut doc, "alpha", &PropValue::number(40.0)).unwrap();
        let Some(Element::SymbolInstance(i)) = element(&doc.editor.document, &TimelineId::Scene(0), &ElementRef::top_level(0, 0, 0)) else { panic!("no instance") };
        let i: &SymbolInstance = i;
        assert!((i.matrix.translation().x - 50.0).abs() < 1e-9);
        assert_eq!(i.name.as_deref(), Some("hero"));
        assert!(matches!(i.color_effect, ColorEffect::Alpha(a) if (a - 0.4).abs() < 1e-6));
        let (rotation, _, _) = linear(i.matrix);
        assert!((rotation - 90.0).abs() < 1e-6);
    }

    #[test]
    fn json_values_follow_the_property_kind() {
        let doc = doc_with_instance();
        assert!(matches!(from_json(&doc, "x", &json!(3.5)), Ok(PropValue::Number { value, .. }) if (value - 3.5).abs() < 1e-9));
        assert!(matches!(from_json(&doc, "blend", &json!("multiply")), Ok(PropValue::Choice { selected: 2, .. })));
        assert!(from_json(&doc, "x", &json!("wide")).is_err());
        assert!(from_json(&doc, "nope", &json!(1)).is_err());
    }
}
