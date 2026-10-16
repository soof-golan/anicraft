//! Shape properties: fill and stroke colour, alpha, width, style, caps, joins and scaling.

use anicraft_tools::WidthProfile;
use anicraft_tools::widthline::apply_profile;
use anim_core::{Rgba8, num};
use anim_display::{Cap, Join, StrokeScale};
use anim_scene::{PrimitiveKind, ShapeElement};
use anim_shape::{FillStyle, OtherStroke, StrokeKind, StrokeStyle};
use anim_ui::property::{PropValue, Property};

use super::{ReadOnly, boolean, choice, colour, number, percent_row, wrong};

const CAPS: [(&str, Cap); 3] = [("None", Cap::None), ("Round", Cap::Round), ("Square", Cap::Square)];
const JOINS: [(&str, Join); 3] = [("Miter", Join::Miter), ("Round", Join::Round), ("Bevel", Join::Bevel)];
const SCALES: [(&str, StrokeScale); 4] =
    [("Normal", StrokeScale::Normal), ("Horizontal", StrokeScale::Horizontal), ("Vertical", StrokeScale::Vertical), ("None", StrokeScale::None)];
const FILL: &str = "Fill";
const STROKE: &str = "Stroke";

/// The stroke styles one can switch to. A shape that already has another pattern keeps it listed, last.
fn style_options(strokes: &[StrokeStyle]) -> Vec<&'static str> {
    let mut options = vec!["Solid", "Dashed", "Dotted"];
    if let Some(StrokeStyle::Other(other)) = strokes.first() {
        match other.kind {
            StrokeKind::Dashed | StrokeKind::Dotted => {}
            StrokeKind::Ragged => options.push("Ragged"),
            StrokeKind::Stipple => options.push("Stipple"),
            StrokeKind::Hatched => options.push("Hatched"),
        }
    }
    options
}

fn style_index(stroke: &StrokeStyle) -> usize {
    match stroke {
        StrokeStyle::Solid(_) => 0,
        StrokeStyle::Other(o) => match o.kind {
            StrokeKind::Dashed => 1,
            StrokeKind::Dotted => 2,
            StrokeKind::Ragged | StrokeKind::Stipple | StrokeKind::Hatched => 3,
        },
    }
}

fn names<T: Copy>(table: &[(&'static str, T)]) -> Vec<&'static str> {
    table.iter().map(|(n, _)| *n).collect()
}

fn fill_kind(fill: &FillStyle) -> &'static str {
    match fill {
        FillStyle::Solid(_) => "Solid",
        FillStyle::LinearGradient(_) => "Linear gradient",
        FillStyle::RadialGradient(_) => "Radial gradient",
        FillStyle::Bitmap(_) => "Bitmap",
    }
}

fn alpha_percent(c: Rgba8) -> f64 {
    f64::from(c.a) / 255.0 * 100.0
}

fn with_alpha_percent(c: Rgba8, percent: f64) -> Rgba8 {
    Rgba8 { a: u8::try_from(num::round_f64_to_u32(percent.clamp(0.0, 100.0) * 2.55)).unwrap_or(255), ..c }
}

/// The rows of a shape.
pub(super) fn describe(s: &ShapeElement, out: &mut Vec<Property>) {
    match s.shape.fills.first() {
        Some(FillStyle::Solid(c)) => {
            out.push(Property::new("fill", "Colour", PropValue::Color(*c)).in_section(FILL));
            out.push(percent_row("fill_alpha", "Alpha", num::f64_to_f32(alpha_percent(*c) / 100.0), FILL));
        }
        Some(other) => {
            out.push(Property::new("fill_type", "Type", PropValue::Text(fill_kind(other).to_owned())).in_section(FILL).read_only());
            if let FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) = other {
                for (i, stop) in g.stops.iter().enumerate() {
                    out.push(
                        Property::new(
                            &format!("fill_stop{i}"),
                            &format!("Stop {} at {:.0}%", i.saturating_add(1), f64::from(stop.offset) * 100.0),
                            PropValue::Color(stop.color),
                        )
                        .in_section(FILL),
                    );
                }
            }
            // Picking a colour turns the fill solid.
            out.push(Property::new("fill", "Replace with colour", PropValue::Color(Rgba8::BLACK)).in_section(FILL));
        }
        None => out.push(Property::new("fill_type", "Type", PropValue::Text("None".to_owned())).in_section(FILL).read_only()),
    }
    match s.shape.strokes.first() {
        Some(stroke) => {
            let base = stroke.solid();
            if let FillStyle::Solid(c) = &base.fill {
                out.push(Property::new("stroke", "Colour", PropValue::Color(*c)).in_section(STROKE));
                out.push(percent_row("stroke_alpha", "Alpha", num::f64_to_f32(alpha_percent(*c) / 100.0), STROKE));
            } else {
                out.push(Property::new("stroke", "Replace with colour", PropValue::Color(Rgba8::BLACK)).in_section(STROKE));
            }
            out.push(
                Property::new("stroke_width", "Width", PropValue::number_in(base.weight, 0.0, 200.0).precision(2, 0.1).with_suffix(" px")).in_section(STROKE),
            );
            out.push(Property::new("stroke_style", "Style", PropValue::choice(style_index(stroke), &style_options(&s.shape.strokes))).in_section(STROKE));
            let profiles: Vec<&str> = WidthProfile::PRESETS.iter().map(WidthProfile::label).collect();
            out.push(Property::new("stroke_profile", "Width profile", PropValue::choice(0, &profiles)).in_section(STROKE));
            let cap = CAPS.iter().position(|(_, c)| *c == base.caps).unwrap_or(1);
            out.push(Property::new("stroke_cap", "Cap", PropValue::choice(cap, &names(&CAPS))).in_section(STROKE));
            let join = JOINS.iter().position(|(_, j)| *j == base.joints).unwrap_or(1);
            out.push(Property::new("stroke_join", "Join", PropValue::choice(join, &names(&JOINS))).in_section(STROKE));
            if base.joints == Join::Miter {
                out.push(Property::new("stroke_miter", "Miter limit", PropValue::number_in(base.miter_limit, 1.0, 255.0).precision(1, 0.1)).in_section(STROKE));
            }
            let scale = SCALES.iter().position(|(_, m)| *m == base.scale_mode).unwrap_or(0);
            out.push(Property::new("stroke_scale", "Scale", PropValue::choice(scale, &names(&SCALES))).in_section(STROKE));
            out.push(Property::new("stroke_hinting", "Stroke hinting", PropValue::Bool(base.pixel_hinting)).in_section(STROKE));
        }
        None => out.push(Property::new("stroke_type", "Type", PropValue::Text("None".to_owned())).in_section(STROKE).read_only()),
    }
    out.push(Property::new("drawing_object", "Object drawing", PropValue::Bool(s.is_drawing_object)).in_section("Mode"));
    if let Some(primitive) = &s.primitive {
        describe_primitive(&primitive.kind, out);
    }
}

const RADII: [(&str, &str); 4] = [
    ("prim_radius_tl", "Top left radius"),
    ("prim_radius_tr", "Top right radius"),
    ("prim_radius_br", "Bottom right radius"),
    ("prim_radius_bl", "Bottom left radius"),
];

fn px_row(id: &str, label: &str, value: f64, max: f64, section: &str) -> Property {
    Property::new(id, label, PropValue::number_in(value, 0.0, max).precision(1, 0.5).with_suffix(" px")).in_section(section)
}

/// The rows of a Rectangle or Oval Primitive.
fn describe_primitive(kind: &PrimitiveKind, out: &mut Vec<Property>) {
    match kind {
        PrimitiveKind::Rectangle(r) => {
            let section = "Rectangle options";
            out.push(px_row("prim_width", "Width", r.width, 16000.0, section));
            out.push(px_row("prim_height", "Height", r.height, 16000.0, section));
            for ((id, label), radius) in RADII.into_iter().zip(r.radii) {
                out.push(px_row(id, label, radius, 1000.0, section));
            }
            out.push(Property::new("prim_lock", "Same radius for all corners", PropValue::Bool(r.lock)).in_section(section));
        }
        PrimitiveKind::Oval(o) => {
            let section = "Oval options";
            out.push(px_row("prim_width", "Width", o.width, 16000.0, section));
            out.push(px_row("prim_height", "Height", o.height, 16000.0, section));
            let degrees = |id: &str, label: &str, value: f64| {
                Property::new(id, label, PropValue::number_in(value, 0.0, 360.0).precision(1, 0.5).with_suffix(" deg")).in_section(section)
            };
            out.push(degrees("prim_start", "Start angle", o.start_angle));
            out.push(degrees("prim_end", "End angle", o.end_angle));
            out.push(
                Property::new("prim_inner", "Inner radius", PropValue::number_in(o.inner_radius, 0.0, 99.0).precision(1, 0.5).with_suffix(" %"))
                    .in_section(section),
            );
            out.push(Property::new("prim_close", "Close path", PropValue::Bool(o.close_path)).in_section(section));
        }
    }
}

/// Applies an edit of a primitive's parameters and regenerates its shape.
fn apply_primitive(s: &mut ShapeElement, id: &str, value: &PropValue) -> Result<(), String> {
    let mut primitive = s.primitive.as_deref().cloned().ok_or_else(|| "this shape is not a primitive".to_owned())?;
    let amount = || number(value).filter(|v| v.is_finite() && *v >= 0.0).ok_or_else(|| wrong(id));
    match (&mut primitive.kind, id) {
        (PrimitiveKind::Rectangle(r), "prim_width") => r.width = amount()?,
        (PrimitiveKind::Rectangle(r), "prim_height") => r.height = amount()?,
        (PrimitiveKind::Rectangle(r), "prim_lock") => {
            r.lock = boolean(value).ok_or_else(|| wrong(id))?;
            if r.lock {
                r.radii = [r.radii.first().copied().unwrap_or(0.0); 4];
            }
        }
        (PrimitiveKind::Rectangle(r), radius) => {
            let index = RADII.iter().position(|(name, _)| *name == radius).ok_or_else(|| format!("rectangle primitives have no property {radius:?}"))?;
            let v = amount()?;
            if r.lock {
                r.radii = [v; 4];
            } else if let Some(slot) = r.radii.get_mut(index) {
                *slot = v;
            }
        }
        (PrimitiveKind::Oval(o), "prim_width") => o.width = amount()?,
        (PrimitiveKind::Oval(o), "prim_height") => o.height = amount()?,
        (PrimitiveKind::Oval(o), "prim_start") => o.start_angle = amount()?,
        (PrimitiveKind::Oval(o), "prim_end") => o.end_angle = amount()?,
        (PrimitiveKind::Oval(o), "prim_inner") => o.inner_radius = amount()?.min(99.0),
        (PrimitiveKind::Oval(o), "prim_close") => o.close_path = boolean(value).ok_or_else(|| wrong(id))?,
        _ => return Err(format!("this primitive has no property {id:?}")),
    }
    let (w, h) = primitive.kind.size();
    s.set_primitive(primitive);
    s.transformation_point = Some(anim_core::Point::new(w / 2.0, h / 2.0));
    Ok(())
}

/// After a style edit the primitive's own fill and stroke follow the shape's.
fn sync_primitive_styles(s: &mut ShapeElement) {
    if let Some(p) = s.primitive.as_mut() {
        p.fill = s.shape.fills.first().cloned();
        p.stroke = s.shape.strokes.first().cloned();
    }
}

/// Applies a shape edit to every fill or stroke the shape has (a shape drawn in merge mode may have several).
pub(super) fn apply(s: &mut ShapeElement, id: &str, value: &PropValue) -> Result<(), String> {
    if id.starts_with("prim_") {
        return apply_primitive(s, id, value);
    }
    apply_style(s, id, value)?;
    sync_primitive_styles(s);
    Ok(())
}

fn apply_style(s: &mut ShapeElement, id: &str, value: &PropValue) -> Result<(), String> {
    match id {
        "fill" => {
            let c = colour(value).ok_or_else(|| wrong(id))?;
            s.shape.fills.fill(FillStyle::Solid(c));
        }
        "fill_alpha" => {
            let percent = number(value).ok_or_else(|| wrong(id))?;
            for fill in &mut s.shape.fills {
                if let FillStyle::Solid(c) = fill {
                    *c = with_alpha_percent(*c, percent);
                }
            }
        }
        "stroke" => {
            let c = colour(value).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                stroke.solid_mut().fill = FillStyle::Solid(c);
            }
        }
        "stroke_alpha" => {
            let percent = number(value).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                if let FillStyle::Solid(c) = &mut stroke.solid_mut().fill {
                    *c = with_alpha_percent(*c, percent);
                }
            }
        }
        "stroke_width" => {
            let w = number(value).filter(|v| *v >= 0.0).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                stroke.solid_mut().weight = w;
            }
        }
        "stroke_style" => set_style(s, choice(value).ok_or_else(|| wrong(id))?)?,
        "stroke_profile" => {
            let profile = WidthProfile::PRESETS.get(choice(value).ok_or_else(|| wrong(id))?).ok_or_else(|| wrong(id))?;
            // The uniform profile is what a stroke already is; the others turn it into a filled outline.
            if *profile != WidthProfile::Uniform {
                s.shape = apply_profile(&s.shape, profile).ok_or_else(|| "the shape has no stroke to give a width profile".to_owned())?;
                // The outline no longer matches the primitive's parameters.
                s.primitive = None;
            }
        }
        "stroke_cap" => {
            let cap = CAPS.get(choice(value).ok_or_else(|| wrong(id))?).map(|(_, c)| *c).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                stroke.solid_mut().caps = cap;
            }
        }
        "stroke_join" => {
            let join = JOINS.get(choice(value).ok_or_else(|| wrong(id))?).map(|(_, j)| *j).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                stroke.solid_mut().joints = join;
            }
        }
        "stroke_miter" => {
            let limit = number(value).filter(|v| (1.0..=255.0).contains(v)).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                stroke.solid_mut().miter_limit = limit;
            }
        }
        "stroke_scale" => {
            let mode = SCALES.get(choice(value).ok_or_else(|| wrong(id))?).map(|(_, m)| *m).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                stroke.solid_mut().scale_mode = mode;
            }
        }
        "stroke_hinting" => {
            let on = boolean(value).ok_or_else(|| wrong(id))?;
            for stroke in &mut s.shape.strokes {
                stroke.solid_mut().pixel_hinting = on;
            }
        }
        "drawing_object" => s.is_drawing_object = boolean(value).ok_or_else(|| wrong(id))?,
        _ => {
            if let Some(index) = id.strip_prefix("fill_stop") {
                return set_stop(s, index, value);
            }
            return Err(format!("shapes have no property {id:?}"));
        }
    }
    Ok(())
}

/// Colour of one gradient stop of the first fill.
fn set_stop(s: &mut ShapeElement, index: &str, value: &PropValue) -> Result<(), String> {
    let c = colour(value).ok_or_else(|| wrong("fill_stop"))?;
    let i: usize = index.parse().map_err(|_| format!("no fill stop {index:?}"))?;
    match s.shape.fills.first_mut() {
        Some(FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g)) => {
            g.stops.get_mut(i).ok_or_else(|| format!("the gradient has no stop {i}"))?.color = c;
            Ok(())
        }
        _ => Err("the fill is not a gradient".to_owned()),
    }
}

/// Switches every stroke to the style at `index` of [`style_options`], keeping weight, colour, caps and joins.
fn set_style(s: &mut ShapeElement, index: usize) -> Result<(), String> {
    let options = style_options(&s.shape.strokes);
    let name = options.get(index).copied().ok_or_else(|| format!("no stroke style number {index}"))?;
    for stroke in &mut s.shape.strokes {
        let kind = match name {
            "Solid" => None,
            "Dashed" => Some((StrokeKind::Dashed, vec![("dash1", "4"), ("gap1", "4")])),
            "Dotted" => Some((StrokeKind::Dotted, vec![("dotSpace", "3")])),
            // Ragged, stipple and hatched are listed only when the shape already has one: keep it as it is.
            _ => continue,
        };
        let same = match (&kind, &*stroke) {
            (None, StrokeStyle::Solid(_)) => true,
            (Some((k, _)), StrokeStyle::Other(o)) => o.kind == *k,
            _ => false,
        };
        if same {
            continue;
        }
        let base = stroke.solid().clone();
        *stroke = match kind {
            None => StrokeStyle::Solid(base),
            Some((kind, attrs)) => {
                StrokeStyle::Other(OtherStroke { kind, base, attrs: attrs.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect() })
            }
        };
    }
    Ok(())
}
