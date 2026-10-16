//! MCP commands for armatures (inverse kinematics): `armature.info`, `armature.add_bone`, `armature.pose`,
//! `armature.set_pose`, `armature.set_limits`, `armature.rename_bone`, `armature.remove_bone` and `armature.remove`.
//!
//! They work on the layer and frame the editor is on (`editor.goto` moves them) unless `layer` / `frame` are given;
//! frames count from 1 like the timeline, angles are degrees, coordinates are stage pixels. Adding a bone runs the
//! Bone tool, so the element and armature kind are chosen exactly as a drag with the mouse would.

use anim_core::{Point, Vec2};
use anim_scene::Pose;
use anim_session::{CommandError, CommandSpec, parse_params};
use anim_timeline::armature::pose_at;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::armature::solve_pose;
use crate::command::ToolCommand;
use crate::editor::Editor;
use crate::mcp::{ToolHost, atomic, down, drag, fail, finite_all, run, up};
use crate::script::ToolScript;
use crate::tool::ToolId;

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::invalid_params(message.into())
}

/// The layer and 0-based frame a command works on.
fn target(editor: &Editor, layer: Option<usize>, frame: Option<u32>) -> Result<(usize, u32), CommandError> {
    let layer = layer.unwrap_or(editor.state.layer);
    let frame = match frame {
        Some(f) => f.checked_sub(1).ok_or_else(|| invalid("frames start at 1"))?,
        None => editor.state.frame,
    };
    let count = editor.document.timeline(&editor.state.timeline).map_or(0, |t| t.layers.len());
    if layer >= count {
        return Err(invalid(format!("the timeline has {count} layers")));
    }
    Ok((layer, frame))
}

/// The armature of `layer` and its pose at `frame`.
fn armature_pose(editor: &Editor, layer: usize, frame: u32) -> Result<(anim_scene::Armature, Pose), CommandError> {
    let l = editor.document.timeline(&editor.state.timeline).and_then(|t| t.layers.get(layer)).ok_or_else(|| invalid("no such layer"))?;
    let armature = l.armature.clone().ok_or_else(|| invalid("the layer has no armature; add a bone first"))?;
    let pose = pose_at(l, frame).unwrap_or_else(|| armature.rest_pose());
    Ok((armature, pose))
}

fn point_json(p: Point) -> Value {
    json!([p.x, p.y])
}

const fn degrees(r: f64) -> f64 {
    r.to_degrees()
}

fn info_json(editor: &Editor, layer: usize, frame: u32) -> Result<Value, CommandError> {
    let (armature, pose) = armature_pose(editor, layer, frame)?;
    let l = editor.document.timeline(&editor.state.timeline).and_then(|t| t.layers.get(layer)).ok_or_else(|| invalid("no such layer"))?;
    let segments = armature.posed_segments(&pose);
    let bones: Vec<Value> = armature
        .bones
        .iter()
        .zip(&segments)
        .enumerate()
        .map(|(i, (b, (head, tip)))| {
            json!({
                "index": i,
                "name": b.name,
                "parent": b.parent,
                "element": b.element,
                "length": b.length,
                "rest_angle_degrees": degrees(b.angle),
                "pose_angle_degrees": degrees(pose.angle(i)),
                "limits_degrees": b.limits.map(|(lo, hi)| [degrees(lo), degrees(hi)]),
                "head": point_json(*head),
                "tip": point_json(*tip),
            })
        })
        .collect();
    let keyframes: Vec<u32> = l.frames.iter().filter(|f| f.pose.is_some()).map(|f| f.index.saturating_add(1)).collect();
    Ok(json!({
        "layer": layer,
        "frame": frame.saturating_add(1),
        "name": armature.name,
        "kind": armature.kind.xfl_name(),
        "offset": point_json(Point::new(pose.offset.x, pose.offset.y)),
        "bones": bones,
        "pose_keyframes": keyframes,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WhereParams {
    layer: Option<usize>,
    frame: Option<u32>,
}

fn armature_info<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: WhereParams = parse_params(params)?;
    let editor = host.editor();
    let (layer, frame) = target(editor, p.layer, p.frame)?;
    info_json(editor, layer, frame)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AddParams {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    /// Parent bone: the new bone starts at its tip.
    parent: Option<usize>,
    name: Option<String>,
    layer: Option<usize>,
    frame: Option<u32>,
}

fn armature_add_bone<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: AddParams = parse_params(params)?;
    finite_all(&[p.x1, p.y1, p.x2, p.y2])?;
    let editor = host.editor_mut();
    let (layer, frame) = target(editor, p.layer, p.frame)?;
    atomic(editor, |ed| {
        ed.state.layer = layer;
        ed.state.frame = frame;
        let before = ed.document.timeline(&ed.state.timeline).and_then(|t| t.layers.get(layer)).and_then(|l| l.armature.as_ref()).map_or(0, |a| a.bones.len());
        let head = match p.parent {
            Some(parent) => {
                let (armature, pose) = armature_pose(ed, layer, frame)?;
                armature.posed_segments(&pose).get(parent).map(|s| s.1).ok_or_else(|| invalid(format!("no bone {parent}")))?
            }
            None => Point::new(p.x1, p.y1),
        };
        let script = ToolScript {
            tool: Some(ToolId::Bone),
            events: vec![down(head.x, head.y), drag(f64::midpoint(head.x, p.x2), f64::midpoint(head.y, p.y2)), up(p.x2, p.y2)],
        };
        run(ed, &script)?;
        let after = ed.document.timeline(&ed.state.timeline).and_then(|t| t.layers.get(layer)).and_then(|l| l.armature.as_ref()).map_or(0, |a| a.bones.len());
        if after != before.saturating_add(1) {
            return Err(invalid(
                "no bone was added: the start point lies on an existing bone (give `parent` to continue from its tip) or the bone is too short",
            ));
        }
        if let Some(name) = &p.name {
            ed.run_commands(&[ToolCommand::RenameBone { layer, bone: before, name: name.clone() }]).map_err(fail)?;
        }
        info_json(ed, layer, frame)
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PoseParams {
    bone: usize,
    x: f64,
    y: f64,
    layer: Option<usize>,
    frame: Option<u32>,
}

fn armature_pose_ik<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: PoseParams = parse_params(params)?;
    finite_all(&[p.x, p.y])?;
    let editor = host.editor_mut();
    let (layer, frame) = target(editor, p.layer, p.frame)?;
    atomic(editor, |ed| {
        let (armature, base) = armature_pose(ed, layer, frame)?;
        if p.bone >= armature.bones.len() {
            return Err(invalid(format!("no bone {}", p.bone)));
        }
        let pose = solve_pose(&armature, &base, p.bone, Point::new(p.x, p.y));
        ed.run_commands(&[ToolCommand::SetPose { layer, frame, pose }]).map_err(fail)?;
        info_json(ed, layer, frame)
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPoseParams {
    /// Rotation of each bone away from its rest angle, degrees; bones past the end of the list are at rest.
    angles_degrees: Vec<f64>,
    offset: Option<[f64; 2]>,
    layer: Option<usize>,
    frame: Option<u32>,
}

fn armature_set_pose<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: SetPoseParams = parse_params(params)?;
    finite_all(&p.angles_degrees)?;
    let editor = host.editor_mut();
    let (layer, frame) = target(editor, p.layer, p.frame)?;
    atomic(editor, |ed| {
        let (armature, base) = armature_pose(ed, layer, frame)?;
        if p.angles_degrees.len() > armature.bones.len() {
            return Err(invalid(format!("the armature has {} bones", armature.bones.len())));
        }
        let mut pose = armature.rest_pose();
        for (i, deg) in p.angles_degrees.iter().enumerate() {
            pose.set_angle(i, deg.to_radians());
        }
        pose.offset = p.offset.map_or(base.offset, |[x, y]| Vec2::new(x, y));
        ed.run_commands(&[ToolCommand::SetPose { layer, frame, pose }]).map_err(fail)?;
        info_json(ed, layer, frame)
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitParams {
    bone: usize,
    /// Both absent removes the limits.
    min_degrees: Option<f64>,
    max_degrees: Option<f64>,
    layer: Option<usize>,
}

fn armature_set_limits<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: LimitParams = parse_params(params)?;
    let editor = host.editor_mut();
    let (layer, frame) = target(editor, p.layer, None)?;
    let limits = match (p.min_degrees, p.max_degrees) {
        (None, None) => None,
        (lo, hi) => {
            let (lo, hi) = (lo.unwrap_or(-180.0), hi.unwrap_or(180.0));
            finite_all(&[lo, hi])?;
            Some((lo.to_radians(), hi.to_radians()))
        }
    };
    atomic(editor, |ed| {
        ed.run_commands(&[ToolCommand::SetBoneLimits { layer, bone: p.bone, limits }]).map_err(fail)?;
        info_json(ed, layer, frame)
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameParams {
    bone: usize,
    name: String,
    layer: Option<usize>,
}

fn armature_rename_bone<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: RenameParams = parse_params(params)?;
    let editor = host.editor_mut();
    let (layer, frame) = target(editor, p.layer, None)?;
    atomic(editor, |ed| {
        ed.run_commands(&[ToolCommand::RenameBone { layer, bone: p.bone, name: p.name.clone() }]).map_err(fail)?;
        info_json(ed, layer, frame)
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoneParams {
    bone: usize,
    layer: Option<usize>,
}

fn armature_remove_bone<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: BoneParams = parse_params(params)?;
    let editor = host.editor_mut();
    let (layer, frame) = target(editor, p.layer, None)?;
    atomic(editor, |ed| {
        ed.run_commands(&[ToolCommand::RemoveBone { layer, bone: p.bone }]).map_err(fail)?;
        let has_armature = ed.document.timeline(&ed.state.timeline).and_then(|t| t.layers.get(layer)).is_some_and(|l| l.armature.is_some());
        if has_armature { info_json(ed, layer, frame) } else { Ok(json!({ "layer": layer, "armature": null })) }
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerParams {
    layer: Option<usize>,
}

fn armature_remove<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: LayerParams = parse_params(params)?;
    let editor = host.editor_mut();
    let (layer, _) = target(editor, p.layer, None)?;
    atomic(editor, |ed| {
        ed.run_commands(&[ToolCommand::RemoveArmature { layer }]).map_err(fail)?;
        Ok(json!({ "layer": layer, "armature": null }))
    })
}

const SCHEMA_WHERE: &str = r#"{"type":"object","properties":{"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based; the playhead by default"}},"additionalProperties":false}"#;
const SCHEMA_ADD: &str = r#"{"type":"object","required":["x1","y1","x2","y2"],"properties":{"x1":{"type":"number","description":"Start (joint) in stage pixels; ignored when parent is given"},"y1":{"type":"number"},"x2":{"type":"number","description":"End (tip) in stage pixels"},"y2":{"type":"number"},"parent":{"type":"integer","minimum":0,"description":"Index of the parent bone; the new bone starts at its tip"},"name":{"type":"string"},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1}},"additionalProperties":false}"#;
const SCHEMA_POSE: &str = r#"{"type":"object","required":["bone","x","y"],"properties":{"bone":{"type":"integer","minimum":0},"x":{"type":"number","description":"Where the bone's tip should go, stage pixels"},"y":{"type":"number"},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1}},"additionalProperties":false}"#;
const SCHEMA_SET_POSE: &str = r#"{"type":"object","required":["angles_degrees"],"properties":{"angles_degrees":{"type":"array","items":{"type":"number"},"description":"Rotation of each bone away from its rest angle, clockwise positive"},"offset":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2,"description":"Translation of the whole armature"},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1}},"additionalProperties":false}"#;
const SCHEMA_LIMITS: &str = r#"{"type":"object","required":["bone"],"properties":{"bone":{"type":"integer","minimum":0},"min_degrees":{"type":"number"},"max_degrees":{"type":"number"},"layer":{"type":"integer","minimum":0}},"additionalProperties":false}"#;
const SCHEMA_RENAME: &str = r#"{"type":"object","required":["bone","name"],"properties":{"bone":{"type":"integer","minimum":0},"name":{"type":"string"},"layer":{"type":"integer","minimum":0}},"additionalProperties":false}"#;
const SCHEMA_BONE: &str = r#"{"type":"object","required":["bone"],"properties":{"bone":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0}},"additionalProperties":false}"#;
const SCHEMA_LAYER: &str = r#"{"type":"object","properties":{"layer":{"type":"integer","minimum":0}},"additionalProperties":false}"#;

/// The armature commands, registered next to the tool commands.
pub fn commands<D: ToolHost>() -> Vec<CommandSpec<D>> {
    vec![
        CommandSpec::new("armature.info", "Armature Info", armature_info::<D>)
            .transient()
            .with_schema(SCHEMA_WHERE)
            .describe("The bones of a layer's armature (names, parents, rest and posed positions, limits, bound elements) and its pose at a frame."),
        CommandSpec::new("armature.add_bone", "Add Bone", armature_add_bone::<D>)
            .with_schema(SCHEMA_ADD)
            .describe("Add a bone with the Bone tool: from (x1, y1) to (x2, y2), or from the tip of `parent`. The first bone turns the layer into an armature layer; the symbol instance under the tip is bound to the bone."),
        CommandSpec::new("armature.pose", "Pose Bone", armature_pose_ik::<D>)
            .with_schema(SCHEMA_POSE)
            .describe("Pose by inverse kinematics: move the tip of a bone towards (x, y) by turning the bones from the root down to it, within the joints' limits. Creates a pose keyframe at the frame."),
        CommandSpec::new("armature.set_pose", "Set Pose", armature_set_pose::<D>)
            .with_schema(SCHEMA_SET_POSE)
            .describe("Set the rotation of every bone at a frame (forward kinematics) and optionally the armature offset. Creates a pose keyframe; poses tween between pose keyframes."),
        CommandSpec::new("armature.set_limits", "Set Joint Limits", armature_set_limits::<D>)
            .with_schema(SCHEMA_LIMITS)
            .describe("Limit how far a bone may rotate away from its rest angle (degrees); give neither bound to remove the limit."),
        CommandSpec::new("armature.rename_bone", "Rename Bone", armature_rename_bone::<D>).with_schema(SCHEMA_RENAME).describe("Rename a bone of a layer's armature; the name shows in the Properties panel and is saved in the file."),
        CommandSpec::new("armature.remove_bone", "Remove Bone", armature_remove_bone::<D>)
            .with_schema(SCHEMA_BONE)
            .describe("Remove a bone and the bones below it."),
        CommandSpec::new("armature.remove", "Remove Armature", armature_remove::<D>)
            .with_schema(SCHEMA_LAYER)
            .describe("Remove the armature of a layer; the layer shows its artwork at rest again."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_valid_json() {
        for spec in commands::<Editor>() {
            let schema = spec.schema_value();
            assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"), "{}", spec.id);
        }
    }
}
