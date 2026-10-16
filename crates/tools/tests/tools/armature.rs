//! The Bone tool, posing with inverse kinematics from the Bone and Selection tools, and the armature commands.
//!
//! Scene (hand-made, predictions in the asserts): two bars of 100 x 20 px lie end to end, the first from (100, 90) to
//! (200, 110) and the second from (200, 90) to (300, 110), so a two-bone arm along y = 100 runs from the shoulder at
//! (100, 100) over the elbow at (200, 100) to the hand at (300, 100). Each bone is 100 px long.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::float_cmp,
    clippy::manual_assert_eq,
    clippy::assert_is_empty,
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    clippy::needless_pass_by_value,
    clippy::disallowed_types
)]

use crate::common::*;
use anicraft_tools::{Editor, ToolCommand, ToolId, mcp_commands};
use anim_core::{Affine, Point};
use anim_scene::{ArmatureKind, Element, Layer, SymbolKind, TimelineId, TweenType, edit};
use anim_session::{CommandRegistry, McpServer, Session};
use anim_timeline::armature::{Posed, pose_at, posed_elements};
use serde_json::{Value, json};

fn arm_editor() -> Editor {
    let mut e = editor();
    e.document.add_symbol("bar", SymbolKind::Graphic).unwrap();
    edit::add_element(&mut e.document, &TimelineId::Symbol("bar".to_owned()), 0, 0, object_rect(0.0, 0.0, 100.0, 20.0, RED)).unwrap();
    for x in [100.0, 200.0] {
        edit::add_symbol_instance(&mut e.document, &TimelineId::Scene(0), 0, 0, "bar", Affine::translate((x, 90.0))).unwrap();
    }
    // Frames 0..24 so there is room for pose keyframes.
    edit::insert_frames(&mut e.document, &TimelineId::Scene(0), 0, 1, 23).unwrap();
    e
}

fn layer(e: &Editor) -> &Layer {
    &e.document.timeline(&e.state.timeline).unwrap().layers[0]
}

fn build_arm(e: &mut Editor) {
    drag(e, ToolId::Bone, (100.0, 100.0), (200.0, 100.0));
    drag(e, ToolId::Bone, (200.0, 100.0), (300.0, 100.0));
}

fn tip_of(e: &Editor, bone: usize, frame: u32) -> Point {
    let l = layer(e);
    let a = l.armature.as_ref().unwrap();
    a.posed_segments(&pose_at(l, frame).unwrap())[bone].1
}

fn translation(m: Affine) -> Point {
    Point::new(m.as_coeffs()[4], m.as_coeffs()[5])
}

#[test]
fn dragging_the_bone_tool_builds_a_bound_two_bone_armature() {
    let mut e = arm_editor();
    build_arm(&mut e);
    let a = layer(&e).armature.as_ref().expect("armature");
    assert_eq!(a.kind, ArmatureKind::Symbol);
    assert_eq!(a.bones.len(), 2);
    assert_eq!((a.bones[0].parent, a.bones[0].element), (None, Some(0)), "the first bone is drawn along the first bar");
    assert_eq!((a.bones[1].parent, a.bones[1].element), (Some(0), Some(1)), "the second starts at the first one's tip and moves the second bar");
    assert!((a.bones[0].length - 100.0).abs() < 1.0e-9 && (a.bones[1].length - 100.0).abs() < 1.0e-9);
    assert!(a.bones[1].head.distance(Point::new(200.0, 100.0)) < 1.0e-9);
    assert!(a.bones[1].angle.abs() < 1.0e-9);
    // The artwork is untouched.
    assert_eq!(translation(elements(&e)[1].matrix().unwrap()), Point::new(200.0, 90.0));
    assert!(e.undo(), "adding a bone is an undo step");
    assert_eq!(layer(&e).armature.as_ref().map_or(1, |a| a.bones.len()), 1);
}

#[test]
fn a_tiny_drag_makes_no_bone() {
    let mut e = arm_editor();
    drag(&mut e, ToolId::Bone, (100.0, 100.0), (101.0, 100.5));
    assert!(layer(&e).armature.is_none());
}

#[test]
fn shift_snaps_the_bone_to_45_degrees() {
    let mut e = arm_editor();
    e.select_tool(ToolId::Bone);
    let shift = anicraft_tools::Modifiers::SHIFT;
    let events = vec![
        anicraft_tools::PointerEvent::down(100.0, 100.0).into(),
        anicraft_tools::PointerEvent::drag(150.0, 120.0).with_modifiers(shift).into(),
        anicraft_tools::PointerEvent::up(150.0, 120.0).with_modifiers(shift).into(),
    ];
    send_all(&mut e, events);
    let a = layer(&e).armature.as_ref().unwrap();
    // (50, 20) is 21.8 degrees below the x axis; snapped it is 0 degrees, keeping the length sqrt(50^2 + 20^2).
    assert!(a.bones[0].angle.abs() < 1.0e-9, "{}", a.bones[0].angle);
    assert!((a.bones[0].length - 50.0_f64.hypot(20.0)).abs() < 1.0e-9);
}

#[test]
fn dragging_a_bone_tip_with_the_selection_tool_poses_with_ik() {
    let mut e = arm_editor();
    build_arm(&mut e);
    // The hand at (300, 100) goes to (200, 200): 141 px from the shoulder, within the 200 px reach.
    drag(&mut e, ToolId::Selection, (300.0, 100.0), (200.0, 200.0));
    let hand = tip_of(&e, 1, 0);
    assert!(hand.distance(Point::new(200.0, 200.0)) < 1.0e-3, "{hand:?}");
    let elbow = tip_of(&e, 0, 0);
    assert!((elbow.distance(Point::new(100.0, 100.0)) - 100.0).abs() < 1.0e-6, "bones keep their length: {elbow:?}");
    // The pose is stored on the first keyframe; the artwork is as drawn.
    assert!(layer(&e).frames[0].pose.is_some());
    assert_eq!(translation(elements(&e)[0].matrix().unwrap()), Point::new(100.0, 90.0));
    // Both bars follow their bones.
    let items = posed_elements(layer(&e), 0);
    let Posed::Element { matrix, .. } = &items[1] else { panic!("element") };
    let moved = *matrix * Point::new(0.0, 10.0);
    assert!(moved.distance(elbow) < 1.0e-6, "the second bar's left edge is at the elbow: {moved:?} vs {elbow:?}");
    // Selection did not start a marquee or move anything: nothing is selected.
    assert!(e.state.selection.is_empty());
    assert!(e.undo(), "posing is undoable");
}

#[test]
fn dragging_a_root_head_moves_the_whole_armature() {
    let mut e = arm_editor();
    build_arm(&mut e);
    drag(&mut e, ToolId::Selection, (100.0, 100.0), (130.0, 80.0));
    let l = layer(&e);
    let pose = pose_at(l, 0).unwrap();
    assert!((pose.offset.x - 30.0).abs() < 1.0e-9 && (pose.offset.y + 20.0).abs() < 1.0e-9, "{:?}", pose.offset);
    assert!(tip_of(&e, 1, 0).distance(Point::new(330.0, 80.0)) < 1.0e-9);
}

#[test]
fn joint_limits_hold_while_dragging() {
    let mut e = arm_editor();
    build_arm(&mut e);
    e.run_commands(&[ToolCommand::SetBoneLimits { layer: 0, bone: 1, limits: Some(((-20.0_f64).to_radians(), 20.0_f64.to_radians())) }]).unwrap();
    drag(&mut e, ToolId::Selection, (300.0, 100.0), (200.0, 200.0));
    let pose = pose_at(layer(&e), 0).unwrap();
    assert!(pose.angle(1).abs() <= 20.0_f64.to_radians() + 1.0e-9, "{}", pose.angle(1).to_degrees());
}

#[test]
fn posing_at_a_later_frame_makes_a_pose_keyframe_that_tweens_from_rest() {
    let mut e = arm_editor();
    build_arm(&mut e);
    e.state.frame = 12;
    drag(&mut e, ToolId::Selection, (300.0, 100.0), (200.0, 200.0));
    let l = layer(&e);
    let indices: Vec<u32> = l.frames.iter().map(|f| f.index).collect();
    assert_eq!(indices, vec![0, 12]);
    assert_eq!(l.frames[0].tween_type, TweenType::Classic, "the artwork keyframe tweens to the pose");
    assert!(l.frames[1].elements.is_empty(), "pose keyframes hold no artwork of their own");
    // Prediction: halfway through the tween, linear ease, every bone has turned half of its final rotation.
    let end = pose_at(l, 12).unwrap();
    let mid = pose_at(l, 6).unwrap();
    for i in 0..2 {
        assert!((mid.angle(i) - end.angle(i) / 2.0).abs() < 1.0e-9, "bone {i}: {} vs {}", mid.angle(i), end.angle(i));
    }
    assert!(pose_at(l, 0).unwrap().angles.iter().all(|a| a.abs() < 1.0e-12), "the first frame is at rest");
    assert!(tip_of(&e, 1, 12).distance(Point::new(200.0, 200.0)) < 1.0e-3);
}

#[test]
fn removing_bones_and_the_armature_restores_a_plain_layer() {
    let mut e = arm_editor();
    build_arm(&mut e);
    e.state.frame = 12;
    drag(&mut e, ToolId::Selection, (300.0, 100.0), (200.0, 200.0));
    e.run_commands(&[ToolCommand::RemoveBone { layer: 0, bone: 1 }]).unwrap();
    assert_eq!(layer(&e).armature.as_ref().unwrap().bones.len(), 1);
    assert_eq!(layer(&e).frames[1].pose.as_ref().unwrap().angles.len(), 1);
    e.run_commands(&[ToolCommand::RemoveArmature { layer: 0 }]).unwrap();
    let l = layer(&e);
    assert!(l.armature.is_none() && l.frames.iter().all(|f| f.pose.is_none()));
    assert_eq!(l.frames.len(), 1, "the pose keyframes are gone and the artwork lasts the whole layer");
    assert_eq!(l.frame_count(), 24);
    assert!(matches!(l.frames[0].elements[0], Element::SymbolInstance(_)));
}

#[test]
fn shape_armatures_deform_the_vector_artwork() {
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 90.0, 300.0, 110.0, RED);
    e.state.frame = 0;
    build_arm(&mut e);
    let a = layer(&e).armature.as_ref().unwrap();
    assert_eq!(a.kind, ArmatureKind::Shape, "vector artwork gets a shape armature");
    assert!(a.bones.iter().all(|b| b.element.is_none()));
    drag(&mut e, ToolId::Selection, (300.0, 100.0), (200.0, 200.0));
    let items = posed_elements(layer(&e), 0);
    let Posed::Shape(shape) = &items[0] else { panic!("a deformed shape") };
    let bounds = shape.bounds().unwrap();
    // The bar bent downwards: it now reaches well below its original 110 px bottom edge.
    assert!(bounds.y1 > 190.0, "{bounds:?}");
}

// ---------------------------------------------------------------------------------------------------------------
// MCP
// ---------------------------------------------------------------------------------------------------------------

struct Client {
    server: McpServer,
    session: Session<Editor>,
    next_id: u64,
}

impl Client {
    fn new() -> Self {
        let mut editor = arm_editor();
        editor.keep_history = false;
        let mut registry = CommandRegistry::with_history_commands();
        registry.register_all(mcp_commands::<Editor>()).unwrap();
        Self { server: McpServer::new("armature-test", "0.1.0"), session: Session::new(editor, registry), next_id: 1 }
    }

    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        self.next_id += 1;
        let message = json!({ "jsonrpc": "2.0", "id": self.next_id, "method": "tools/call", "params": { "name": name, "arguments": arguments } });
        let reply = self.server.handle(&mut self.session, &message).expect("a response");
        let result = &reply["result"];
        assert!(result.is_object(), "{reply}");
        let is_error = result["isError"].as_bool().unwrap();
        if let Some(structured) = result.get("structuredContent") {
            return (is_error, structured.clone());
        }
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        (is_error, serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned())))
    }

    fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (is_error, value) = self.call(name, arguments);
        assert!(!is_error, "{name}: {value}");
        value
    }

    fn err(&mut self, name: &str, arguments: Value) -> String {
        let (is_error, value) = self.call(name, arguments);
        assert!(is_error, "{name} should fail: {value}");
        value.to_string()
    }
}

fn point(v: &Value) -> Point {
    Point::new(v[0].as_f64().unwrap(), v[1].as_f64().unwrap())
}

#[test]
fn armatures_are_driven_over_mcp_with_undo() {
    let mut c = Client::new();
    assert!(c.err("armature_info", json!({})).contains("no armature"));
    let info = c.ok("armature_add_bone", json!({ "x1": 100, "y1": 100, "x2": 200, "y2": 100, "name": "upper" }));
    assert_eq!(info["kind"], "symbol");
    assert_eq!(info["bones"][0]["name"], "upper");
    assert_eq!(info["bones"][0]["element"], 0);
    let info = c.ok("armature_add_bone", json!({ "x1": 0, "y1": 0, "x2": 300, "y2": 100, "parent": 0, "name": "lower" }));
    assert_eq!(info["bones"][1]["parent"], 0);
    assert!((point(&info["bones"][1]["head"]).distance(Point::new(200.0, 100.0))) < 1.0e-9);
    assert!(c.err("armature_add_bone", json!({ "x1": 100, "y1": 100, "x2": 100, "y2": 100 })).contains("no bone was added"));

    let posed = c.ok("armature_pose", json!({ "bone": 1, "x": 200, "y": 200 }));
    assert!(point(&posed["bones"][1]["tip"]).distance(Point::new(200.0, 200.0)) < 1.0e-3, "{posed}");
    assert_eq!(posed["pose_keyframes"], json!([1]));
    assert!(c.err("armature_pose", json!({ "bone": 9, "x": 1, "y": 1 })).contains("no bone 9"));

    let at_rest = c.ok("armature_set_pose", json!({ "angles_degrees": [0, 0], "frame": 13 }));
    assert_eq!(at_rest["pose_keyframes"], json!([1, 13]));
    assert!(point(&at_rest["bones"][1]["tip"]).distance(Point::new(300.0, 100.0)) < 1e-9);
    let quarter = c.ok("armature_set_pose", json!({ "angles_degrees": [90], "frame": 13 }));
    assert!(point(&quarter["bones"][1]["tip"]).distance(Point::new(100.0, 300.0)) < 1e-9, "{quarter}");
    assert!(c.err("armature_set_pose", json!({ "angles_degrees": [1, 2, 3] })).contains("2 bones"));

    let limited = c.ok("armature_set_limits", json!({ "bone": 0, "min_degrees": -10, "max_degrees": 10 }));
    assert_eq!(limited["bones"][0]["limits_degrees"], json!([-10.0, 10.0]));
    let clamped = c.ok("armature_set_pose", json!({ "angles_degrees": [90], "frame": 13 }));
    assert!((clamped["bones"][0]["pose_angle_degrees"].as_f64().unwrap() - 10.0).abs() < 1.0e-9);
    let unlimited = c.ok("armature_set_limits", json!({ "bone": 0 }));
    assert_eq!(unlimited["bones"][0]["limits_degrees"], Value::Null);

    c.ok("armature_rename_bone", json!({ "bone": 1, "name": "forearm" }));
    assert_eq!(c.ok("armature_info", json!({}))["bones"][1]["name"], "forearm");

    // Undo walks the edits back.
    c.ok("edit_undo", json!({}));
    c.ok("edit_undo", json!({}));
    assert_eq!(c.ok("armature_info", json!({}))["bones"][1]["name"], "lower");

    let after_remove = c.ok("armature_remove_bone", json!({ "bone": 1 }));
    assert_eq!(after_remove["bones"].as_array().unwrap().len(), 1);
    c.ok("armature_remove", json!({}));
    assert!(c.err("armature_info", json!({})).contains("no armature"));
}
