//! Timelines, layers, frames and the elements inside them.
//!
//! Every function here mirrors a reader in `timeline.rs` or `element.rs`: attributes the reader would default are
//! left out, everything else is written, and `extra` goes last so unmodelled content survives.

use anim_core::{Affine, Point, Rgba8};
use anim_display::Filter;
use anim_scene::{
    BitmapInstance, ColorEffect, EaseCurve, Element, Frame, FrameSound, Group, InstanceFilter, Layer, LoopMode, SoundLoop, SoundSync, SymbolInstance,
    SymbolKind, TextElement, TextRun, Timeline,
};

use super::out::{Attrs, XmlOut};

/// Deepest group nesting written. The reader turns anything deeper into raw XML, so the model never holds more.
const MAX_GROUP_DEPTH: usize = anim_scene::MAX_GROUP_DEPTH;

/// `<matrix><Matrix a b c d tx ty/></matrix>` with the identity's components left out.
pub fn matrix(out: &mut XmlOut, m: Affine) {
    let [xx, yx, xy, yy, tx, ty] = m.as_coeffs();
    out.open("matrix", &Attrs::new());
    let attrs = Attrs::new()
        .f_unless("a", xx, 1.0)
        .f_unless("b", yx, 0.0)
        .f_unless("c", xy, 0.0)
        .f_unless("d", yy, 1.0)
        .f_unless("tx", tx, 0.0)
        .f_unless("ty", ty, 0.0);
    out.leaf("Matrix", &attrs);
    out.close();
}

/// A bare `<Point x y/>`.
pub fn point_element(out: &mut XmlOut, p: Point) {
    out.leaf("Point", &Attrs::new().f_unless("x", p.x, 0.0).f_unless("y", p.y, 0.0));
}

/// `<tag><Point x y/></tag>`.
pub fn point(out: &mut XmlOut, tag: &'static str, p: Point) {
    out.open(tag, &Attrs::new());
    point_element(out, p);
    out.close();
}

/// Writes a `<DOMTimeline>` (a scene, or the timeline of a symbol).
pub fn timeline(out: &mut XmlOut, t: &Timeline) {
    let attrs = Attrs::new().s("name", &t.name).u_unless("currentFrame", t.current_frame, 0).b_opt("layerDepthEnabled", t.layer_depth_enabled).extra(&t.extra);
    out.open("DOMTimeline", &attrs);
    out.open("layers", &Attrs::new());
    for layer in &t.layers {
        write_layer(out, layer);
    }
    out.close();
    for child in &t.extra.children {
        out.raw("DOMTimeline", child);
    }
    out.close();
}

fn write_layer(out: &mut XmlOut, layer: &Layer) {
    let attrs = Attrs::new()
        .s("name", &layer.name)
        .color("color", layer.color)
        .s_opt("layerType", layer.kind.xfl_name())
        .u_opt("parentLayerIndex", layer.parent_layer_index.and_then(|p| u32::try_from(p).ok()))
        .b_unless("locked", layer.locked, false)
        .b_unless("visible", layer.visible, true)
        .b_unless("outline", layer.outline, false)
        .b_unless("autoNamed", layer.autoname, true)
        .extra(&layer.extra);
    out.open("DOMLayer", &attrs);
    out.open("frames", &Attrs::new());
    for frame in &layer.frames {
        write_frame(out, frame);
    }
    out.close();
    for child in &layer.extra.children {
        out.raw("DOMLayer", child);
    }
    out.close();
}

fn write_frame(out: &mut XmlOut, frame: &Frame) {
    let has_custom_ease = frame.eases.iter().any(|e| matches!(e.curve, EaseCurve::Custom(_)));
    let mut attrs = Attrs::new()
        .u("index", frame.index)
        .u_unless("duration", frame.duration, 1)
        .s_opt("tweenType", frame.tween_type.xfl_name())
        .u("keyMode", frame.key_mode)
        .s_opt("motionTweenRotate", frame.motion_tween_rotate.xfl_name())
        .f_opt("motionTweenRotateTimes", frame.motion_tween_rotate_times)
        .b_unless("motionTweenScale", frame.motion_tween_scale, true)
        .b_unless("motionTweenSnap", frame.motion_tween_snap, false)
        .b_unless("motionTweenOrientToPath", frame.motion_tween_orient_to_path, false)
        .b_unless("motionTweenSync", frame.motion_tween_sync, false)
        .i_unless("acceleration", frame.acceleration, 0)
        .s_opt("easeMethodName", frame.ease_method_name.as_deref())
        .b_unless("hasCustomEase", has_custom_ease, false)
        .s_opt("name", frame.label.as_deref())
        .s_opt("labelType", frame.label_type.map(anim_scene::LabelType::xfl_name));
    if let Some(sound) = &frame.sound {
        attrs = sound_attrs(attrs, sound);
    }
    let attrs = attrs.extra(&frame.extra);
    out.open("DOMFrame", &attrs);
    if let Some(script) = &frame.actionscript {
        out.open("Actionscript", &Attrs::new());
        out.cdata_leaf("script", script);
        out.close();
    }
    if let Some(sound) = &frame.sound
        && !sound.envelope.is_empty()
    {
        out.open("SoundEnvelope", &Attrs::new());
        for p in &sound.envelope {
            out.leaf("SoundEnvelopePoint", &Attrs::new().u_unless("mark44", p.mark44, 0).u("level0", p.level0).u("level1", p.level1));
        }
        out.close();
    }
    if let Some(xml) = &frame.motion_object_xml {
        out.raw("motionObjectXML", xml);
    }
    if let Some(xml) = &frame.morph_shape_xml {
        out.raw("MorphShape", xml);
    } else if !frame.shape_hints.is_empty() {
        out.warnings.push(format!("frame {}: shape hints are not written without a MorphShape element", frame.index));
    }
    if !frame.eases.is_empty() {
        write_tweens(out, frame);
    }
    out.open("elements", &Attrs::new());
    for element in &frame.elements {
        write_element(out, element, 0);
    }
    out.close();
    for child in &frame.extra.children {
        out.raw("DOMFrame", child);
    }
    out.close();
}

fn sound_attrs<'a>(attrs: Attrs<'a>, sound: &'a FrameSound) -> Attrs<'a> {
    let (loop_mode, repeat) = match sound.loop_mode {
        SoundLoop::Once => (None, None),
        SoundLoop::Repeat(n) => (Some("repeat"), Some(n)),
        SoundLoop::Forever => (Some("loop"), None),
    };
    attrs
        .s("soundName", &sound.name)
        .s_opt("soundSync", (sound.sync != SoundSync::Event).then(|| sound.sync.xfl_name()))
        .s_opt("soundLoopMode", loop_mode)
        .u_opt("repeatCount", repeat)
        .s_opt("soundEffect", sound.effect.as_deref())
        .extra(&sound.extra)
}

fn write_tweens(out: &mut XmlOut, frame: &Frame) {
    out.open("tweens", &Attrs::new());
    for ease in &frame.eases {
        match &ease.curve {
            EaseCurve::Named(method) => out.leaf("Ease", &Attrs::new().s("target", ease.target.xfl_name()).s("method", method)),
            EaseCurve::Custom(points) => {
                out.open("CustomEase", &Attrs::new().s("target", ease.target.xfl_name()));
                for [x, y] in points {
                    point_element(out, Point::new(*x, *y));
                }
                out.close();
            }
        }
    }
    out.close();
}

/// Writes one element of a keyframe or group.
pub fn write_element(out: &mut XmlOut, element: &Element, depth: usize) {
    match element {
        Element::Shape(shape) => super::shape::write_shape(out, shape),
        Element::SymbolInstance(instance) => symbol_instance(out, instance),
        Element::Bitmap(bitmap) => bitmap_instance(out, bitmap),
        Element::Text(text) => write_text(out, text),
        Element::Group(group) => {
            if depth >= MAX_GROUP_DEPTH {
                out.warnings.push(format!("a group nested more than {MAX_GROUP_DEPTH} levels deep was left out"));
            } else {
                write_group(out, group, depth);
            }
        }
        Element::Unknown(raw) => out.raw(&raw.tag, &raw.xml),
    }
}

fn write_group(out: &mut XmlOut, group: &Group, depth: usize) {
    out.open("DOMGroup", &Attrs::new().extra(&group.extra));
    if group.matrix != Affine::IDENTITY {
        matrix(out, group.matrix);
    }
    if let Some(p) = group.transformation_point {
        point(out, "transformationPoint", p);
    }
    out.open("members", &Attrs::new());
    for member in &group.members {
        write_element(out, member, depth.saturating_add(1));
    }
    out.close();
    for child in &group.extra.children {
        out.raw("DOMGroup", child);
    }
    out.close();
}

fn bitmap_instance(out: &mut XmlOut, bitmap: &BitmapInstance) {
    out.open("DOMBitmapInstance", &Attrs::new().s("libraryItemName", &bitmap.library_item_name).extra(&bitmap.extra));
    matrix(out, bitmap.matrix);
    for child in &bitmap.extra.children {
        out.raw("DOMBitmapInstance", child);
    }
    out.close();
}

fn symbol_instance(out: &mut XmlOut, i: &SymbolInstance) {
    let attrs = Attrs::new()
        .s("libraryItemName", &i.library_item_name)
        .s_opt("symbolType", i.symbol_type.map(SymbolKind::xfl_name))
        .s_opt("name", i.name.as_deref())
        .s_opt("loop", i.loop_mode.map(LoopMode::xfl_name))
        .u_unless("firstFrame", i.first_frame, 0)
        .u_opt("lastFrame", i.last_frame)
        .s_opt("blendMode", i.blend_mode.xfl_name())
        .b_unless("isVisible", i.visible, true)
        .b_unless("cacheAsBitmap", i.cache_as_bitmap, false)
        .extra(&i.extra);
    out.open("DOMSymbolInstance", &attrs);
    if i.matrix != Affine::IDENTITY {
        matrix(out, i.matrix);
    }
    point(out, "transformationPoint", i.transformation_point);
    color_effect(out, i.color_effect);
    if !i.filters.is_empty() {
        out.open("filters", &Attrs::new());
        for filter in &i.filters {
            write_filter(out, filter);
        }
        out.close();
    }
    for child in &i.extra.children {
        out.raw("DOMSymbolInstance", child);
    }
    out.close();
}

/// Offsets are stored divided by 255; undo the division and drop the float noise.
fn offset_text(unit: f32) -> f64 {
    (f64::from(unit) * 255.0 * 1000.0).round() / 1000.0
}

fn color_effect(out: &mut XmlOut, effect: ColorEffect) {
    let attrs = match effect {
        ColorEffect::None => return,
        ColorEffect::Brightness(b) => Attrs::new().f32("brightness", b),
        ColorEffect::Tint { color, amount } => Attrs::new().f32("tintMultiplier", amount).color("tintColor", color),
        ColorEffect::Alpha(a) => Attrs::new().f32("alphaMultiplier", a),
        ColorEffect::Advanced(t) => {
            let [mr, mg, mb, ma] = t.mul;
            let [ar, ag, ab, aa] = t.add;
            let plain = |v: f32| same(v, 1.0);
            let no_offsets = [ar, ag, ab, aa].iter().all(|v| same(*v, 0.0));
            // An alpha-only transform would read back as the simple Alpha style, so name the red multiplier then.
            let force_red = plain(mr) && plain(mg) && plain(mb) && no_offsets;
            let mut attrs = Attrs::new();
            if !plain(mr) || force_red {
                attrs = attrs.f32("redMultiplier", mr);
            }
            if !plain(mg) {
                attrs = attrs.f32("greenMultiplier", mg);
            }
            if !plain(mb) {
                attrs = attrs.f32("blueMultiplier", mb);
            }
            if !plain(ma) {
                attrs = attrs.f32("alphaMultiplier", ma);
            }
            for (name, v) in [("redOffset", ar), ("greenOffset", ag), ("blueOffset", ab), ("alphaOffset", aa)] {
                if !same(v, 0.0) {
                    attrs = attrs.f(name, offset_text(v));
                }
            }
            attrs
        }
    };
    out.open("color", &Attrs::new());
    out.leaf("Color", &attrs);
    out.close();
}

fn same(a: f32, b: f32) -> bool {
    (a - b).abs() <= f32::EPSILON * 4.0
}

fn write_filter(out: &mut XmlOut, filter: &InstanceFilter) {
    match filter {
        InstanceFilter::Raw(raw) => out.raw(&raw.tag, &raw.xml),
        InstanceFilter::Mapped { filter, extra } => {
            let (tag, attrs): (&'static str, Attrs<'_>) = match filter {
                Filter::Blur { x, y, quality } => ("BlurFilter", Attrs::new().f("blurX", *x).f("blurY", *y).u_unless("quality", u32::from(*quality), 1)),
                Filter::DropShadow { color, angle_deg, distance, blur_x, blur_y, strength, inner, knockout, hide_object } => (
                    "DropShadowFilter",
                    Attrs::new()
                        .f("blurX", *blur_x)
                        .f("blurY", *blur_y)
                        .f("angle", *angle_deg)
                        .f("distance", *distance)
                        .f("strength", *strength)
                        .color_unless("color", *color, Rgba8::BLACK)
                        .alpha("alpha", *color)
                        .b_unless("inner", *inner, false)
                        .b_unless("knockout", *knockout, false)
                        .b_unless("hideObject", *hide_object, false),
                ),
                Filter::Glow { color, blur_x, blur_y, strength, inner, knockout } => (
                    "GlowFilter",
                    Attrs::new()
                        .f("blurX", *blur_x)
                        .f("blurY", *blur_y)
                        .f("strength", *strength)
                        .color("color", *color)
                        .alpha("alpha", *color)
                        .b_unless("inner", *inner, false)
                        .b_unless("knockout", *knockout, false),
                ),
                Filter::ColorMatrix(_) => {
                    out.warnings.push("a colour-matrix filter has no XFL element and was left out".to_owned());
                    return;
                }
            };
            out.open(tag, &attrs.extra(extra));
            for child in &extra.children {
                out.raw(tag, child);
            }
            out.close();
        }
    }
}

fn write_text(out: &mut XmlOut, text: &TextElement) {
    let attrs = Attrs::new()
        .f("width", text.width)
        .f("height", text.height)
        .b_unless("autoExpand", text.auto_expand, false)
        .s_opt("orientation", text.orientation.xfl_name())
        .s_opt("instanceName", text.instance_name.as_deref())
        .extra(&text.extra);
    let tag = text.kind.element_name();
    out.open(tag, &attrs);
    matrix(out, text.matrix);
    out.open("textRuns", &Attrs::new());
    for run in &text.text_runs {
        write_run(out, run);
    }
    out.close();
    for child in &text.extra.children {
        out.raw(tag, child);
    }
    out.close();
}

fn write_run(out: &mut XmlOut, run: &TextRun) {
    out.open("DOMTextRun", &Attrs::new());
    out.text_leaf("characters", &Attrs::new(), &run.characters);
    out.open("textAttrs", &Attrs::new());
    let attrs = Attrs::new()
        .f32("size", run.size)
        .b_unless("bold", run.bold, false)
        .b_unless("italic", run.italic, false)
        .s_unless("face", &run.face, "")
        .color("fillColor", run.fill_color)
        .f32_unless("alpha", run.alpha, 1.0)
        .f32_unless("letterSpacing", run.letter_spacing, 0.0)
        .s_unless("alignment", run.alignment.xfl_name(), "left")
        .extra(&run.extra);
    out.leaf("DOMTextAttrs", &attrs);
    out.close();
    for child in &run.extra.children {
        out.raw("DOMTextRun", child);
    }
    out.close();
}
