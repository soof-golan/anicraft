//! A posed armature exports to SWF as plain per-frame transforms.
//!
//! Prediction (100 x 100 stage): a red bar x 10..50, y 45..55 hangs on bone 0 (joint (10, 50)) and a green bar
//! x 50..90 on its child bone 1. Frame 0 is the rest pose, so no placement is rotated. Frame 5 turns bone 0 a
//! quarter turn clockwise, so the red bar is placed with a matrix whose rotate-skew terms are plus or minus one
//! (20 twips per pixel does not enter the 16.16 matrix terms), and the movie has six frames.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::panic)]

use anicraft_swf::export::{ExportOptions, export_display_lists};
use anim_core::{Point, Rgba8};
use anim_scene::{Armature, ArmatureBone, ArmatureKind, Document, Element, Frame, Layer, Pose, ShapeElement, TweenType};
use anim_shape::{EdgeShape, FillStyle, PathInput};
use anim_timeline::Evaluator;
use std::f64::consts::FRAC_PI_2;

fn bar(x: f64, color: Rgba8) -> Element {
    let mut path = anim_core::BezPath::new();
    path.move_to(Point::new(x, 45.0));
    path.line_to(Point::new(x + 40.0, 45.0));
    path.line_to(Point::new(x + 40.0, 55.0));
    path.line_to(Point::new(x, 55.0));
    path.close_path();
    Element::Shape(ShapeElement::new(EdgeShape::from_paths(&[PathInput::new(path, Some(FillStyle::Solid(color)), None)])))
}

fn document() -> Document {
    let mut doc = Document::new(100.0, 100.0, 12.0);
    let mut layer = Layer::new("rig", Rgba8::opaque(1, 2, 3));
    let mut armature = Armature::new("rig", ArmatureKind::Symbol);
    let mut b0 = ArmatureBone::between("a", None, Point::new(10.0, 50.0), Point::new(50.0, 50.0));
    b0.element = Some(0);
    armature.push(b0).unwrap();
    let mut b1 = ArmatureBone::between("b", Some(0), Point::new(50.0, 50.0), Point::new(90.0, 50.0));
    b1.element = Some(1);
    armature.push(b1).unwrap();
    layer.armature = Some(armature);
    let mut rest = Frame::new(0, 5, vec![bar(10.0, Rgba8::opaque(255, 0, 0)), bar(50.0, Rgba8::opaque(0, 255, 0))]);
    rest.pose = Some(Pose::rest(2));
    rest.tween_type = TweenType::Classic;
    let mut posed = Frame::new(5, 1, Vec::new());
    let mut pose = Pose::rest(2);
    pose.set_angle(0, FRAC_PI_2);
    posed.pose = Some(pose);
    layer.frames.push(rest);
    layer.frames.push(posed);
    doc.scenes[0].layers.clear();
    doc.scenes[0].layers.push(layer);
    doc
}

/// The rotate-skew terms of every placement matrix in each frame of the movie, as `(b, c)` magnitudes.
fn skews_per_frame(bytes: &[u8]) -> Vec<Vec<f32>> {
    let buf = swf::decompress_swf(bytes).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    let mut frames = vec![Vec::new()];
    for tag in &parsed.tags {
        match tag {
            swf::Tag::PlaceObject(place) => {
                if let Some(m) = place.matrix {
                    frames.last_mut().unwrap().push(m.b.to_f32().abs().max(m.c.to_f32().abs()));
                }
            }
            swf::Tag::ShowFrame => frames.push(Vec::new()),
            _ => {}
        }
    }
    frames.pop();
    frames
}

#[test]
fn a_posed_armature_exports_as_per_frame_transforms() {
    let doc = document();
    let mut ev = Evaluator::new(&doc);
    let lists: Vec<_> = (0..6).map(|f| ev.frame(0, f)).collect();
    let out = export_display_lists(&lists, ev.images(), ExportOptions::for_list(&lists[0], 12.0)).unwrap();
    let frames = skews_per_frame(&out.swf);
    assert_eq!(frames.len(), 6);
    assert!(frames[0].iter().all(|s| *s < 1.0e-3), "rest pose is unrotated: {:?}", frames[0]);
    assert!(frames[5].iter().any(|s| (*s - 1.0).abs() < 1.0e-2), "frame 5 is turned a quarter: {:?}", frames[5]);
}
