//! `ActionScript` queries: a keyframe's script with its diagnostics, and a headless Test Movie.

use anim_mcp::ControlError;
use anim_scene::Document;
use anim_script::Player;
use serde_json::{Value, json};

use super::{scene_of, uint};

/// Most frames one `test_movie` call may advance.
const MAX_TICKS: u64 = 20_000;

/// The script of the keyframe covering `frame` of `layer`, with what the parser could not read.
pub(super) fn frame_script(doc: &Document, params: &Value) -> Result<Value, ControlError> {
    let scene = scene_of(doc, params.get("scene").and_then(Value::as_u64).and_then(|s| usize::try_from(s).ok()))?;
    let timeline = doc.scenes.get(scene).ok_or_else(|| ControlError::invalid("no such scene"))?;
    let layer = uint(params, "layer")?;
    let frame = uint(params, "frame")?.checked_sub(1).ok_or_else(|| ControlError::invalid("frames start at 1"))?;
    let layer_ref = timeline.layers.get(layer).ok_or_else(|| ControlError::invalid(format!("no layer {layer}")))?;
    let key = layer_ref
        .keyframe_at(u32::try_from(frame).map_err(|_| ControlError::invalid("frame out of range"))?)
        .ok_or_else(|| ControlError::invalid(format!("layer {layer} has no keyframe covering frame {}", frame.saturating_add(1))))?;
    let script = key.actionscript.clone().unwrap_or_default();
    let parsed = anim_script::parse(&script);
    Ok(json!({
        "scene": scene,
        "layer": layer,
        "keyframe": key.index.saturating_add(1),
        "version": format!("{:?}", anim_script::script_version(doc)),
        "script": script,
        "diagnostics": parsed.diagnostics.iter().map(|d| json!({ "line": d.line, "message": d.message })).collect::<Vec<_>>(),
    }))
}

/// Runs the document in the preview runtime without a window: starts the movie, then performs `steps` in order and
/// reports where the playhead is and what `trace` wrote.
///
/// A step is `{ "tick": n }` (advance `n` frames) or `{ "event": "click", "target": "instanceName" }`.
pub(super) fn test_movie(doc: &Document, params: &Value) -> Result<Value, ControlError> {
    let steps = params.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut player = Player::new(doc);
    let diagnostics: Vec<Value> = player
        .diagnostics()
        .into_iter()
        .map(|(scene, layer, frame, line, message)| {
            json!({ "scene": scene, "layer": layer, "frame": frame.saturating_add(1), "line": line, "message": message })
        })
        .collect();
    player.start(doc);
    let mut ticks = 0_u64;
    let mut trail = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        if let Some(n) = step.get("tick").and_then(Value::as_u64) {
            ticks = ticks.saturating_add(n);
            if ticks > MAX_TICKS {
                return Err(ControlError::invalid(format!("a test movie may advance at most {MAX_TICKS} frames")));
            }
            for _ in 0..n {
                player.advance(doc);
            }
        } else if let Some(event) = step.get("event").and_then(Value::as_str) {
            let target = step.get("target").and_then(Value::as_str).ok_or_else(|| ControlError::invalid(format!("step {i}: `target` names the instance")))?;
            player.dispatch(doc, target, event);
        } else {
            return Err(ControlError::invalid(format!("step {i} is neither a `tick` nor an `event`")));
        }
        let (scene, frame) = player.position();
        trail.push(json!({ "step": i, "scene": scene, "frame": frame.saturating_add(1), "playing": player.is_playing() }));
    }
    let (scene, frame) = player.position();
    let output = player.take_output();
    Ok(json!({
        "version": format!("{:?}", player.version()),
        "scene": scene,
        "frame": frame.saturating_add(1),
        "playing": player.is_playing(),
        "output": output,
        "diagnostics": diagnostics,
        "trail": trail,
        "supported": "timeline control (stop, play, gotoAndPlay, gotoAndStop, nextFrame, prevFrame, nextScene, prevScene, frame labels), button event handlers (addEventListener with MouseEvent, onPress and friends), variables, if/else, functions, trace",
    }))
}
