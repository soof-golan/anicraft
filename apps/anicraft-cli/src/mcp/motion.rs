//! Motion tweens, motion presets, shape hints and custom eases as commands.
//!
//! The same conventions as [`super::commands`]: layers are 0-based, frames are 1-based, `scene` defaults to the
//! timeline the editor is in. A motion tween's keys are addressed by `offset`, the number of frames after the start
//! of the span (the span is the keyframe's whole length). Every command is one undo step and a failure changes
//! nothing.

use anim_core::Point;
use anim_scene::hints;
use anim_scene::motion::{self, KeyEase, MotionProp};
use anim_scene::timeline::EaseTarget;
use anim_scene::{SceneError, TimelineId};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use serde::Deserialize;
use serde_json::{Value, json};

use super::commands::{timeline, zero_based};
use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

fn failed(e: &SceneError) -> CommandError {
    CommandError::failed(e.to_string())
}

/// Registers [`all`] into `registry`.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(all()).expect("motion command ids are unique");
}

/// The `property` schema member.
macro_rules! prop {
    () => {
        r#""property":{"enum":["x","y","rotation","scale_x","scale_y","alpha"],"description":"x and y in stage pixels, rotation in degrees, scales and alpha in percent"}"#
    };
}

/// An object schema with `scene`, `layer` and `frame` plus `$props` (comma-led members), requiring `$required`.
macro_rules! schema {
    ($required:literal, $props:expr) => {
        concat!(
            r#"{"type":"object","additionalProperties":false,"required":["#,
            $required,
            r#"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based, any frame of the tween"}"#,
            $props,
            "}}"
        )
    };
}

/// The commands of this module.
#[must_use]
#[allow(clippy::too_many_lines, reason = "one spec per command")]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("timeline.create_motion_tween", "Create Motion Tween", create_motion_tween)
            .describe("Turns the keyframe at `frame` into a motion tween over its whole span (at least two frames; insert frames first). The keyframe must hold one symbol instance, group or text field. Edit it afterwards with the `motion.*` commands.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","frame"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1}}}"#),
        CommandSpec::new("motion.set_key", "Set Property Keyframe", set_key)
            .describe("Adds or changes a property keyframe of a motion tween: `offset` frames after the span start the object shows `value` for `property`. Offset 0 is the object itself and cannot be set; move the object instead.")
            .with_schema(schema!(r#""layer","frame","property","offset","value""#, concat!(",", prop!(), r#","offset":{"type":"integer","minimum":1},"value":{"type":"number"}"#))),
        CommandSpec::new("motion.remove_key", "Remove Property Keyframe", remove_key)
            .describe("Removes the property keyframe `offset` frames into a motion tween.")
            .with_schema(schema!(r#""layer","frame","property","offset""#, concat!(",", prop!(), r#","offset":{"type":"integer","minimum":1}"#))),
        CommandSpec::new("motion.set_key_ease", "Set Keyframe Ease", set_key_ease)
            .describe("Sets the ease of the segment that starts at a property keyframe: a classic `acceleration` (-100 ease in .. 100 ease out), or a custom `curve` of 1+3k bezier points [time, value] from [0,0] to [1,1], which wins.")
            .with_schema(schema!(
                r#""layer","frame","property","offset""#,
                concat!(
                    ",",
                    prop!(),
                    r#","offset":{"type":"integer","minimum":0},"acceleration":{"type":"integer","minimum":-100,"maximum":100},"curve":{"type":"array","items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}}"#
                )
            )),
        CommandSpec::new("motion.set_path_point", "Move Motion Path Point", set_path_point)
            .describe("Sets x and y together at `offset` frames into a motion tween: one point of the motion path.")
            .with_schema(schema!(r#""layer","frame","offset","x","y""#, r#","offset":{"type":"integer","minimum":1},"x":{"type":"number"},"y":{"type":"number"}"#)),
        CommandSpec::new("motion.apply_preset", "Apply Motion Preset", apply_preset)
            .describe("Replaces the tracks of a motion tween with a built-in preset: fly-in-left, fly-in-top, fade-in, fade-out, spin, pulse, bounce, wiggle, zoom-in. Presets that start away from the rest position move the object's own keyframe.")
            .with_schema(schema!(r#""layer","frame","preset""#, r#","preset":{"enum":["fly-in-left","fly-in-top","fade-in","fade-out","spin","pulse","bounce","wiggle","zoom-in"]}"#)),
        CommandSpec::new("shape.add_hint", "Add Shape Hint", add_hint)
            .describe("Adds the next free shape hint letter (a to z) to the shape tween at `frame`, at the centre of the start and end shapes. Returns its `hint` number (0 = a). Move it with `shape.move_hint`.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","frame"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1}}}"#),
        CommandSpec::new("shape.move_hint", "Move Shape Hint", move_hint)
            .describe("Moves the start point (`end` false, shown on the start keyframe) or the end point (`end` true) of a shape hint to stage position (x, y).")
            .with_schema(schema!(r#""layer","frame","hint","x","y""#, r#","hint":{"type":"integer","minimum":0,"maximum":25},"end":{"type":"boolean","default":false},"x":{"type":"number"},"y":{"type":"number"}"#)),
        CommandSpec::new("shape.remove_hint", "Remove Shape Hint", remove_hint)
            .describe("Removes one shape hint, or every hint of the tween when `hint` is omitted.")
            .with_schema(schema!(r#""layer","frame""#, r#","hint":{"type":"integer","minimum":0,"maximum":25}"#)),
        CommandSpec::new("timeline.set_custom_ease", "Set Custom Ease", set_custom_ease)
            .describe("Sets a custom ease curve on a tween: 1+3k bezier points [time, value] from [0,0] to [1,1] for `target` (all, position, rotation, scale, color, filters; default all). Without `curve` the custom ease is removed.")
            .with_schema(schema!(
                r#""layer","frame""#,
                r#","target":{"enum":["all","position","rotation","scale","color","filters"],"default":"all"},"curve":{"type":["array","null"],"items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}}"#
            )),
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct At {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
}

fn resolve(doc: &AniDoc, p: &At) -> Result<(TimelineId, u32), CommandError> {
    Ok((timeline(doc, p.scene)?, zero_based(p.frame)?))
}

fn property(name: &str) -> Result<MotionProp, CommandError> {
    MotionProp::from_label(name).ok_or_else(|| CommandError::invalid_params(format!("unknown property {name:?}")))
}

/// The `motion_tween` query: what the keyframe covering `frame` of `layer` tweens.
pub(super) fn query(doc: &anim_scene::Document, params: &Value) -> Result<Value, anim_mcp::ControlError> {
    use anim_mcp::ControlError;
    let (_, timeline) = super::timeline_of(doc, params)?;
    let layer = super::uint(params, "layer")?;
    let frame = super::uint(params, "frame")?.checked_sub(1).ok_or_else(|| ControlError::invalid("frames start at 1"))?;
    let keyframe = timeline
        .layers
        .get(layer)
        .ok_or_else(|| ControlError::invalid(format!("no layer {layer}")))?
        .keyframe_at(u32::try_from(frame).map_err(|_| ControlError::invalid("frame out of range"))?)
        .ok_or_else(|| ControlError::invalid(format!("layer {layer} has no keyframe covering frame {}", frame.saturating_add(1))))?;
    let steps = keyframe.duration.saturating_sub(1);
    let tracks: Vec<Value> = motion::motion_object(keyframe)
        .map(|object| {
            object
                .tracks
                .iter()
                .map(|t| {
                    let keys: Vec<Value> = t
                        .keys
                        .iter()
                        .map(|k| json!({ "offset": (k.t * f64::from(steps)).round(), "value": k.value, "ease": k.ease, "curve": k.curve }))
                        .collect();
                    json!({ "property": t.prop.label(), "keys": keys })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "tween": format!("{:?}", keyframe.tween_type),
        "start_frame": keyframe.index.saturating_add(1),
        "span_frames": keyframe.duration,
        "tracks": tracks,
        "shape_hints": keyframe.shape_hints.iter().map(|h| json!({ "hint": h.id, "start": [h.start.x, h.start.y], "end": [h.end.x, h.end.y] })).collect::<Vec<_>>(),
        "custom_eases": keyframe.eases.iter().filter_map(|e| match &e.curve { anim_scene::EaseCurve::Custom(p) => Some(json!({ "target": e.target.xfl_name(), "curve": p })), anim_scene::EaseCurve::Named(_) => None }).collect::<Vec<_>>(),
        "presets": motion::MOTION_PRESETS.iter().map(|p| json!({ "name": p.name, "description": p.description })).collect::<Vec<_>>(),
    }))
}

fn create_motion_tween(doc: &mut AniDoc, params: &Value) -> Out {
    let p: At = parse_params(params)?;
    let (id, frame) = resolve(doc, &p)?;
    motion::create_motion_tween(&mut doc.editor.document, &id, p.layer, frame).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.layer, "frame": p.frame }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Key {
    #[serde(flatten)]
    at: At,
    property: String,
    offset: u32,
    value: f64,
}

fn set_key(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Key = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    motion::set_motion_key(&mut doc.editor.document, &id, p.at.layer, frame, property(&p.property)?, p.offset, p.value).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame, "property": p.property, "offset": p.offset }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyAt {
    #[serde(flatten)]
    at: At,
    property: String,
    offset: u32,
}

fn remove_key(doc: &mut AniDoc, params: &Value) -> Out {
    let p: KeyAt = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    motion::remove_motion_key(&mut doc.editor.document, &id, p.at.layer, frame, property(&p.property)?, p.offset).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame, "property": p.property, "offset": p.offset }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyEaseParams {
    #[serde(flatten)]
    at: At,
    property: String,
    offset: u32,
    acceleration: Option<i32>,
    curve: Option<Vec<[f64; 2]>>,
}

fn set_key_ease(doc: &mut AniDoc, params: &Value) -> Out {
    let p: KeyEaseParams = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    let ease = KeyEase { acceleration: p.acceleration.unwrap_or(0), curve: p.curve };
    motion::set_motion_key_ease(&mut doc.editor.document, &id, p.at.layer, frame, property(&p.property)?, p.offset, ease).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame, "property": p.property, "offset": p.offset }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathPoint {
    #[serde(flatten)]
    at: At,
    offset: u32,
    x: f64,
    y: f64,
}

fn set_path_point(doc: &mut AniDoc, params: &Value) -> Out {
    let p: PathPoint = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    let mut copy = doc.editor.document.clone();
    motion::set_motion_key(&mut copy, &id, p.at.layer, frame, MotionProp::X, p.offset, p.x).map_err(|e| failed(&e))?;
    motion::set_motion_key(&mut copy, &id, p.at.layer, frame, MotionProp::Y, p.offset, p.y).map_err(|e| failed(&e))?;
    doc.editor.document = copy;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame, "offset": p.offset }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Preset {
    #[serde(flatten)]
    at: At,
    preset: String,
}

fn apply_preset(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Preset = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    motion::apply_motion_preset(&mut doc.editor.document, &id, p.at.layer, frame, &p.preset).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame, "preset": p.preset }))
}

fn add_hint(doc: &mut AniDoc, params: &Value) -> Out {
    let p: At = parse_params(params)?;
    let (id, frame) = resolve(doc, &p)?;
    let hint = hints::add_shape_hint(&mut doc.editor.document, &id, p.layer, frame).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.layer, "frame": p.frame, "hint": hint }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveHint {
    #[serde(flatten)]
    at: At,
    hint: u32,
    #[serde(default)]
    end: bool,
    x: f64,
    y: f64,
}

fn move_hint(doc: &mut AniDoc, params: &Value) -> Out {
    let p: MoveHint = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    hints::move_shape_hint(&mut doc.editor.document, &id, p.at.layer, frame, p.hint, p.end, Point::new(p.x, p.y)).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame, "hint": p.hint }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoveHint {
    #[serde(flatten)]
    at: At,
    hint: Option<u32>,
}

fn remove_hint(doc: &mut AniDoc, params: &Value) -> Out {
    let p: RemoveHint = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    hints::remove_shape_hint(&mut doc.editor.document, &id, p.at.layer, frame, p.hint).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomEase {
    #[serde(flatten)]
    at: At,
    target: Option<String>,
    curve: Option<Vec<[f64; 2]>>,
}

fn set_custom_ease(doc: &mut AniDoc, params: &Value) -> Out {
    let p: CustomEase = parse_params(params)?;
    let (id, frame) = resolve(doc, &p.at)?;
    let target = match p.target.as_deref() {
        None => EaseTarget::All,
        Some(name) => EaseTarget::from_xfl_name(name).ok_or_else(|| CommandError::invalid_params(format!("unknown ease target {name:?}")))?,
    };
    hints::set_custom_ease(&mut doc.editor.document, &id, p.at.layer, frame, target, p.curve).map_err(|e| failed(&e))?;
    Ok(json!({ "layer": p.at.layer, "frame": p.at.frame }))
}

#[cfg(test)]
mod tests {
    use anim_core::Affine;
    use anim_scene::{Element, Frame, SymbolInstance};
    use anim_session::Session;

    use super::*;

    fn session() -> Session<AniDoc> {
        let mut doc = AniDoc::default();
        let instance = SymbolInstance::new("Ball", Affine::translate((10.0, 20.0)));
        if let Some(layer) = doc.editor.document.scenes.first_mut().and_then(|s| s.layers.first_mut()) {
            layer.frames = vec![Frame::new(0, 11, vec![Element::SymbolInstance(Box::new(instance))])];
        }
        Session::new(doc, super::super::registry())
    }

    #[test]
    fn every_schema_is_valid_json() {
        for spec in all() {
            serde_json::from_str::<Value>(spec.input_schema.unwrap()).unwrap_or_else(|e| panic!("{}: {e}", spec.id));
        }
    }

    #[test]
    fn motion_tween_lifecycle_is_undoable() {
        let mut s = session();
        s.execute("timeline.create_motion_tween", &json!({ "layer": 0, "frame": 1 })).unwrap();
        s.execute("motion.set_key", &json!({ "layer": 0, "frame": 1, "property": "x", "offset": 10, "value": 210 })).unwrap();
        s.execute("motion.set_path_point", &json!({ "layer": 0, "frame": 1, "offset": 5, "x": 100, "y": 150 })).unwrap();
        s.execute("motion.set_key_ease", &json!({ "layer": 0, "frame": 1, "property": "x", "offset": 10, "acceleration": -50 })).unwrap();
        s.execute("motion.apply_preset", &json!({ "layer": 0, "frame": 1, "preset": "spin" })).unwrap();
        assert!(s.execute("motion.set_key", &json!({ "layer": 0, "frame": 1, "property": "x", "offset": 0, "value": 1 })).is_err());
        assert!(s.execute("motion.set_key", &json!({ "layer": 0, "frame": 1, "property": "nope", "offset": 3, "value": 1 })).is_err());
        let before = s.doc().editor.document.clone();
        s.undo();
        assert_ne!(s.doc().editor.document, before);
    }

    #[test]
    fn shape_hints_and_custom_eases_are_commands() {
        let mut s = session();
        if let Some(layer) = s.doc_mut().editor.document.scenes.first_mut().and_then(|sc| sc.layers.first_mut()) {
            let mut start = Frame::new(0, 5, vec![]);
            start.tween_type = anim_scene::TweenType::Shape;
            layer.frames = vec![start, Frame::new(5, 1, vec![])];
        }
        let added = s.execute("shape.add_hint", &json!({ "layer": 0, "frame": 1 })).unwrap();
        assert_eq!(added["hint"], 0);
        s.execute("shape.move_hint", &json!({ "layer": 0, "frame": 1, "hint": 0, "end": true, "x": 40, "y": 50 })).unwrap();
        let hint = s.doc().editor.document.scenes[0].layers[0].frames[0].shape_hints[0];
        assert_eq!((hint.end.x, hint.end.y), (40.0, 50.0));
        s.execute("timeline.set_custom_ease", &json!({ "layer": 0, "frame": 1, "curve": [[0, 0], [0.3, 0], [0.7, 1], [1, 1]] })).unwrap();
        assert!(s.execute("timeline.set_custom_ease", &json!({ "layer": 0, "frame": 1, "curve": [[0, 0], [1, 1]] })).is_err());
        s.execute("shape.remove_hint", &json!({ "layer": 0, "frame": 1 })).unwrap();
        assert_eq!(s.doc().editor.document.scenes[0].layers[0].frames[0].shape_hints.len(), 0);
        assert!(s.execute("shape.remove_hint", &json!({ "layer": 0, "frame": 1, "hint": 4 })).is_err());
    }
}
