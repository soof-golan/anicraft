//! The layer and the keyframe under the playhead: layer state, frame label, tween type, easing and tween options.

use anim_core::num;
use anim_scene::edit;
use anim_scene::timeline::{EaseCurve, EaseTarget, LabelType, LayerKind, TweenEase, TweenRotate, TweenType};
use anim_scene::{Document, Frame, TimelineId};
use anim_ui::property::{PropValue, Property};

use super::{ReadOnly, Unit, boolean, choice, colour, number, number_row, text, wrong};

const LAYER: &str = "Layer";
const FRAME: &str = "Frame";
const TWEEN: &str = "Tween";
const LABELS: [(&str, LabelType); 3] = [("Name", LabelType::Name), ("Comment", LabelType::Comment), ("Anchor", LabelType::Anchor)];
const TWEENS: [&str; 3] = ["None", "Classic", "Shape"];
const ROTATES: [(&str, TweenRotate); 4] =
    [("Auto", TweenRotate::Auto), ("None", TweenRotate::None), ("Clockwise", TweenRotate::Clockwise), ("Counter-clockwise", TweenRotate::CounterClockwise)];
const FAMILIES: [&str; 10] = ["sine", "quad", "cubic", "quart", "quint", "expo", "circ", "back", "elastic", "bounce"];
const ACCELERATION: Unit = Unit::new(-100.0, 100.0, 0, "");
const TURNS: Unit = Unit::new(0.0, 100.0, 0, "");

/// The ease choices: the classic slider, linear and every preset (`quadIn`, `bounceInOut`, ...).
fn ease_names() -> Vec<String> {
    let mut names = vec!["Classic (slider)".to_owned(), "linear".to_owned()];
    for family in FAMILIES {
        for mode in ["In", "Out", "InOut"] {
            names.push(format!("{family}{mode}"));
        }
    }
    names
}

fn layer_kind_name(kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::Normal => "Normal",
        LayerKind::Guide => "Guide",
        LayerKind::Guided => "Guided",
        LayerKind::Mask => "Mask",
        LayerKind::Masked => "Masked",
        LayerKind::Folder => "Folder",
        LayerKind::Camera => "Camera",
    }
}

/// The layer and keyframe rows for layer `layer` at `frame` (both 0-based) of timeline `id`.
pub(super) fn describe(doc: &Document, id: &TimelineId, layer: usize, frame: u32, out: &mut Vec<Property>) {
    let Some(timeline) = doc.timeline(id) else { return };
    let Some(l) = timeline.layers.get(layer) else { return };
    out.push(Property::new("layer.name", "Name", PropValue::Text(l.name.clone())).in_section(LAYER));
    out.push(Property::new("layer.visible", "Visible", PropValue::Bool(l.visible)).in_section(LAYER));
    out.push(Property::new("layer.locked", "Locked", PropValue::Bool(l.locked)).in_section(LAYER));
    out.push(Property::new("layer.outline", "Outline", PropValue::Bool(l.outline)).in_section(LAYER));
    out.push(Property::new("layer.color", "Outline colour", PropValue::Color(l.color)).in_section(LAYER));
    let kind = timeline.resolved_kind(layer).unwrap_or(l.kind);
    out.push(Property::new("layer.kind", "Type", PropValue::Text(layer_kind_name(kind).to_owned())).in_section(LAYER).read_only());
    let Some(k) = l.keyframe_at(frame) else {
        out.push(Property::new("frame.info", "Frame", PropValue::Text(format!("{} (no keyframe)", frame.saturating_add(1)))).in_section(FRAME).read_only());
        return;
    };
    let span = format!("{} to {} ({} frame{})", k.index.saturating_add(1), k.end(), k.duration, if k.duration == 1 { "" } else { "s" });
    out.push(Property::new("frame.span", "Keyframe", PropValue::Text(span)).in_section(FRAME).read_only());
    out.push(Property::new("frame.label", "Label", PropValue::Text(k.label.clone().unwrap_or_default())).in_section(FRAME));
    if k.label.is_some() {
        let label_type = LABELS.iter().position(|(_, t)| Some(*t) == k.label_type).unwrap_or(0);
        let label_names: Vec<&str> = LABELS.iter().map(|(n, _)| *n).collect();
        out.push(Property::new("frame.label_type", "Label type", PropValue::choice(label_type, &label_names)).in_section(FRAME));
    }
    describe_tween(k, out);
}

fn describe_tween(k: &Frame, out: &mut Vec<Property>) {
    let mut tweens = TWEENS.to_vec();
    let current = match k.tween_type {
        TweenType::None => 0,
        TweenType::Classic => 1,
        TweenType::Shape => 2,
        TweenType::MotionObject => {
            tweens.push("Motion");
            3
        }
    };
    out.push(Property::new("tween.type", "Type", PropValue::choice(current, &tweens)).in_section(TWEEN));
    if matches!(k.tween_type, TweenType::None | TweenType::MotionObject) {
        return;
    }
    let mut names = ease_names();
    let custom = k.ease_for(EaseTarget::All).is_some_and(|c| matches!(c, EaseCurve::Custom(_)));
    let selected = match k.ease_for(EaseTarget::All) {
        Some(EaseCurve::Named(n)) => names.iter().position(|x| x.eq_ignore_ascii_case(n)).unwrap_or(0),
        Some(EaseCurve::Custom(_)) => {
            names.push("Custom curve".to_owned());
            names.len().saturating_sub(1)
        }
        None => 0,
    };
    let options: Vec<&str> = names.iter().map(String::as_str).collect();
    out.push(Property::new("tween.preset", "Ease preset", PropValue::choice(selected, &options)).in_section(TWEEN));
    if selected == 0 && !custom {
        out.push(number_row("tween.ease", "Ease", f64::from(k.acceleration), ACCELERATION, TWEEN));
    }
    if k.tween_type == TweenType::Classic {
        let rotate = ROTATES.iter().position(|(_, r)| *r == k.motion_tween_rotate).unwrap_or(0);
        let rotate_names: Vec<&str> = ROTATES.iter().map(|(n, _)| *n).collect();
        out.push(Property::new("tween.rotate", "Rotate", PropValue::choice(rotate, &rotate_names)).in_section(TWEEN));
        if matches!(k.motion_tween_rotate, TweenRotate::Clockwise | TweenRotate::CounterClockwise) {
            out.push(number_row("tween.rotate_times", "Rotate times", k.motion_tween_rotate_times.unwrap_or(1.0), TURNS, TWEEN));
        }
        out.push(Property::new("tween.scale", "Scale", PropValue::Bool(k.motion_tween_scale)).in_section(TWEEN));
        out.push(Property::new("tween.snap", "Snap", PropValue::Bool(k.motion_tween_snap)).in_section(TWEEN));
        out.push(Property::new("tween.orient", "Orient to path", PropValue::Bool(k.motion_tween_orient_to_path)).in_section(TWEEN));
        out.push(Property::new("tween.sync", "Sync graphics", PropValue::Bool(k.motion_tween_sync)).in_section(TWEEN));
    }
}

/// Applies a `layer.*`, `frame.*` or `tween.*` edit to layer `layer` at `frame`.
pub(super) fn apply(doc: &mut Document, id: &TimelineId, layer: usize, frame: u32, key: &str, value: &PropValue) -> Result<(), String> {
    let wrong = || wrong(key);
    if let Some(field) = key.strip_prefix("layer.") {
        return apply_layer(doc, id, layer, field, value);
    }
    let missing = || format!("layer {layer} has no keyframe at frame {}", frame.saturating_add(1));
    match key {
        "frame.label" => {
            let label = text(value).ok_or_else(wrong)?;
            let kind = doc.timeline(id).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).and_then(|k| k.label_type).unwrap_or_default();
            let new = (!label.is_empty()).then_some((label.as_str(), kind));
            edit::set_frame_label(doc, id, layer, frame, new).map_err(|e| e.to_string())
        }
        "frame.label_type" => {
            let kind = LABELS.get(choice(value).ok_or_else(wrong)?).map(|(_, t)| *t).ok_or_else(wrong)?;
            let k = edit::keyframe_mut(doc, id, layer, frame).map_err(|_| missing())?;
            if k.label.is_none() {
                return Err("give the keyframe a label first".to_owned());
            }
            k.label_type = Some(kind);
            Ok(())
        }
        "tween.type" => set_tween_type(doc, id, layer, frame, choice(value).ok_or_else(wrong)?),
        _ => {
            let k = edit::keyframe_mut(doc, id, layer, frame).map_err(|_| missing())?;
            if matches!(k.tween_type, TweenType::None) {
                return Err("there is no tween on this keyframe; set tween.type first".to_owned());
            }
            apply_tween(k, key, value)
        }
    }
}

fn apply_layer(doc: &mut Document, id: &TimelineId, layer: usize, field: &str, value: &PropValue) -> Result<(), String> {
    let wrong = || wrong(&format!("layer.{field}"));
    if field == "name" {
        return edit::rename_layer(doc, id, layer, &text(value).ok_or_else(wrong)?).map_err(|e| e.to_string());
    }
    let l = doc.timeline_mut(id).and_then(|t| t.layers.get_mut(layer)).ok_or_else(|| format!("no layer {layer}"))?;
    match field {
        "visible" => l.visible = boolean(value).ok_or_else(wrong)?,
        "locked" => l.locked = boolean(value).ok_or_else(wrong)?,
        "outline" => l.outline = boolean(value).ok_or_else(wrong)?,
        "color" => l.color = colour(value).ok_or_else(wrong)?,
        _ => return Err(format!("layers have no property {field:?}")),
    }
    Ok(())
}

fn set_tween_type(doc: &mut Document, id: &TimelineId, layer: usize, frame: u32, which: usize) -> Result<(), String> {
    let current = doc.timeline(id).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).map(|k| k.tween_type);
    let wanted = match which {
        0 => TweenType::None,
        1 => TweenType::Classic,
        2 => TweenType::Shape,
        3 if current == Some(TweenType::MotionObject) => return Ok(()),
        3 => return Err("motion tweens are created with the Create Motion Tween command".to_owned()),
        _ => return Err(format!("no tween type number {which}")),
    };
    if current == Some(wanted) {
        return Ok(());
    }
    match wanted {
        TweenType::None => edit::remove_tween(doc, id, layer, frame),
        TweenType::Classic => edit::create_classic_tween(doc, id, layer, frame),
        TweenType::Shape => edit::create_shape_tween(doc, id, layer, frame),
        TweenType::MotionObject => return Ok(()),
    }
    .map_err(|e| e.to_string())
}

fn apply_tween(k: &mut Frame, key: &str, value: &PropValue) -> Result<(), String> {
    let wrong = || wrong(key);
    match key {
        "tween.preset" => {
            let names = ease_names();
            let index = choice(value).ok_or_else(wrong)?;
            if index == 0 {
                k.eases.retain(|e| e.target != EaseTarget::All);
                k.ease_method_name = None;
            } else if let Some(name) = names.get(index) {
                k.eases.retain(|e| e.target != EaseTarget::All);
                k.eases.push(TweenEase { target: EaseTarget::All, curve: EaseCurve::Named(name.clone()) });
                k.acceleration = 0;
                k.ease_method_name = None;
            } else if k.ease_for(EaseTarget::All).is_none_or(|c| !matches!(c, EaseCurve::Custom(_))) {
                return Err(format!("no ease preset number {index}"));
            }
        }
        "tween.ease" => {
            let acceleration = number(value).filter(|v| (-100.0..=100.0).contains(v)).ok_or_else(wrong)?;
            k.acceleration = num::round_f64_to_i32(acceleration);
            k.eases.retain(|e| e.target != EaseTarget::All);
            k.ease_method_name = None;
        }
        "tween.rotate" => k.motion_tween_rotate = ROTATES.get(choice(value).ok_or_else(wrong)?).map(|(_, r)| *r).ok_or_else(wrong)?,
        "tween.rotate_times" => k.motion_tween_rotate_times = Some(number(value).filter(|v| (0.0..=100.0).contains(v)).ok_or_else(wrong)?),
        "tween.scale" => k.motion_tween_scale = boolean(value).ok_or_else(wrong)?,
        "tween.snap" => k.motion_tween_snap = boolean(value).ok_or_else(wrong)?,
        "tween.orient" => k.motion_tween_orient_to_path = boolean(value).ok_or_else(wrong)?,
        "tween.sync" => k.motion_tween_sync = boolean(value).ok_or_else(wrong)?,
        _ => return Err(format!("no property {key:?}")),
    }
    Ok(())
}
