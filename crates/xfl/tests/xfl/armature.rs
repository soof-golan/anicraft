//! Armature layers survive XFL writing and reading, and hand-written XFL with bones reads as predicted.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::assert_is_empty,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test code"
)]

use anicraft_xfl::{ReadOptions, WriteOptions, read_bytes, write_fla_bytes};
use anim_core::{Affine, Point, Rgba8, Vec2};
use anim_scene::{Armature, ArmatureBone, ArmatureKind, Document, Element, Frame, Layer, MediaStore, Pose, SymbolInstance, SymbolKind, TweenType};

fn document() -> Document {
    let mut doc = Document::new(400.0, 300.0, 24.0);
    doc.add_symbol("limb", SymbolKind::Graphic).unwrap();
    let mut layer = Layer::new("Armature_1", Rgba8::new(0x4f, 0x80, 0xff, 255));
    let mut armature = Armature::new("Armature_1", ArmatureKind::Symbol);
    let mut upper = ArmatureBone::between("upper", None, Point::new(100.0, 100.0), Point::new(200.0, 100.0));
    upper.element = Some(0);
    upper.limits = Some(((-45.0_f64).to_radians(), 90.0_f64.to_radians()));
    armature.push(upper).unwrap();
    let mut lower = ArmatureBone::between("lower", Some(0), Point::new(200.0, 100.0), Point::new(200.0, 180.0));
    lower.element = Some(1);
    armature.push(lower).unwrap();
    layer.armature = Some(armature);
    let rest = Frame::new(
        0,
        12,
        vec![
            Element::SymbolInstance(Box::new(SymbolInstance::new("limb", Affine::translate((100.0, 100.0))))),
            Element::SymbolInstance(Box::new(SymbolInstance::new("limb", Affine::translate((200.0, 100.0))))),
        ],
    );
    let mut rest = rest;
    rest.pose = Some(Pose::rest(2));
    rest.tween_type = TweenType::Classic;
    let mut posed = Frame::new(12, 12, Vec::new());
    posed.pose = Some(Pose { offset: Vec2::new(5.0, -3.0), angles: vec![30.0_f64.to_radians(), -60.0_f64.to_radians()] });
    layer.frames.push(rest);
    layer.frames.push(posed);
    doc.scenes[0].layers.clear();
    doc.scenes[0].layers.push(layer);
    doc
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1.0e-9
}

#[test]
fn armature_layers_round_trip() {
    let doc = document();
    let (bytes, _) = write_fla_bytes(&doc, &MediaStore::new(), &[], &WriteOptions::default()).unwrap();
    let back = read_bytes(bytes, &ReadOptions::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let layer = &back.document.scenes[0].layers[0];
    let armature = layer.armature.as_ref().expect("armature");
    assert_eq!(armature.name, "Armature_1");
    assert_eq!(armature.kind, ArmatureKind::Symbol);
    assert_eq!(armature.bones.len(), 2);
    let upper = &armature.bones[0];
    assert_eq!((upper.name.as_str(), upper.parent, upper.element), ("upper", None, Some(0)));
    assert!(close(upper.length, 100.0) && close(upper.angle, 0.0));
    let (lo, hi) = upper.limits.expect("limits");
    assert!(close(lo.to_degrees(), -45.0) && close(hi.to_degrees(), 90.0));
    let lower = &armature.bones[1];
    assert_eq!((lower.parent, lower.element, lower.limits), (Some(0), Some(1), None));
    assert!(close(lower.angle.to_degrees(), 90.0), "the lower bone points down the stage: {}", lower.angle.to_degrees());
    assert!(lower.head.distance(Point::new(200.0, 100.0)) < 1.0e-9);

    assert_eq!(layer.frames.len(), 2);
    assert_eq!(layer.frames[0].elements.len(), 2);
    assert!(layer.frames[1].elements.is_empty());
    let pose = layer.frames[1].pose.as_ref().expect("pose");
    assert!(close(pose.angle(0).to_degrees(), 30.0) && close(pose.angle(1).to_degrees(), -60.0));
    assert!(close(pose.offset.x, 5.0) && close(pose.offset.y, -3.0));
    assert_eq!(layer.frames[0].tween_type, TweenType::Classic);
}

#[test]
fn writing_an_armature_twice_is_a_fixed_point() {
    let doc = document();
    let first = write_fla_bytes(&doc, &MediaStore::new(), &[], &WriteOptions::default()).unwrap().0;
    let loaded = read_bytes(first, &ReadOptions::default()).unwrap();
    let second = write_fla_bytes(&loaded.document, &loaded.media, &loaded.sidecars, &WriteOptions::default()).unwrap().0;
    let again = read_bytes(second.clone(), &ReadOptions::default()).unwrap();
    let third = write_fla_bytes(&again.document, &again.media, &again.sidecars, &WriteOptions::default()).unwrap().0;
    assert_eq!(second, third);
    assert_eq!(loaded.document.scenes[0].layers[0].armature, again.document.scenes[0].layers[0].armature);
}

#[test]
fn hand_written_bones_read_as_predicted() {
    // Prediction: a root bone 100 px long pointing right, a child pointing down; the child may rotate -10..10 degrees.
    let xml = r##"<DOMDocument width="300" height="200" frameRate="24"><timelines><DOMTimeline name="Scene 1"><layers>
      <DOMLayer name="rig" color="#4F80FF"><armature name="arm" kind="shape" custom="kept">
        <Bone name="a" x="10" y="20" length="100" angle="0" />
        <Bone name="b" parent="0" x="110" y="20" length="50" angle="90" minAngle="-10" maxAngle="10" mystery="1" />
        <Bone name="c" parent="7" x="0" y="0" length="1" angle="nan" />
      </armature><frames>
        <DOMFrame index="0" duration="3"><pose angles="0 5 bad"/><elements/></DOMFrame>
      </frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"##;
    let bytes = {
        let mut writer = anim_archive::ZipWriter::with_mimetype(Vec::new(), "application/vnd.adobe.xfl", anim_archive::WriteOptions::default()).unwrap();
        anim_archive::ArchiveWriter::add_file(&mut writer, "DOMDocument.xml", xml.as_bytes()).unwrap();
        anim_archive::ArchiveWriter::finish(writer).unwrap()
    };
    let loaded = read_bytes(bytes, &ReadOptions::default()).unwrap();
    let layer = &loaded.document.scenes[0].layers[0];
    let armature = layer.armature.as_ref().expect("armature");
    assert_eq!(armature.kind, ArmatureKind::Shape);
    assert_eq!(armature.extra.attr("custom"), Some("kept"));
    assert_eq!(armature.bones.len(), 3);
    assert!(close(armature.bones[1].angle.to_degrees(), 90.0));
    assert_eq!(armature.bones[1].extra.attr("mystery"), Some("1"));
    let (lo, hi) = armature.bones[1].limits.unwrap();
    assert!(close(lo.to_degrees(), -10.0) && close(hi.to_degrees(), 10.0));
    assert_eq!(armature.bones[2].parent, None, "a parent that does not precede the bone is dropped");
    assert!(armature.is_well_formed());
    assert!(!loaded.warnings.is_empty(), "the bad parent and the bad angle are reported");
    let pose = layer.frames[0].pose.as_ref().unwrap();
    assert_eq!(pose.angles.len(), 3);
    assert!(close(pose.angle(1).to_degrees(), 5.0));
    assert_eq!(pose.angle(2), 0.0);
}
