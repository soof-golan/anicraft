//! The shape commands end to end: run them on a session, write an `.fla`, read it back and check what survived.
//!
//! With `ANICRAFT_SHOT_DIR` set the test also leaves the documents it builds in that folder, for the application's
//! `--screenshot`.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    reason = "test code"
)]

use std::path::PathBuf;

use anicraft_cli::mcp::{AniDoc, registry};
use anicraft_cli::project::Project;
use anicraft_tools::lock::is_locked;
use anicraft_tools::modify::fill_area;
use anicraft_xfl::WriteOptions;
use anim_scene::Element;
use anim_session::Session;
use serde_json::json;

fn session() -> Session<AniDoc> {
    Session::new(AniDoc::default(), registry())
}

fn rect(s: &mut Session<AniDoc>, x: f64, y: f64, w: f64, h: f64, fill: &str) {
    s.execute(
        "draw.rectangle",
        &json!({ "x": x, "y": y, "width": w, "height": h, "fill": fill, "stroke": { "color": "#222222", "width": 3 }, "object_drawing": true }),
    )
    .unwrap();
}

fn oval(s: &mut Session<AniDoc>, x: f64, y: f64, w: f64, h: f64, fill: &str) {
    s.execute(
        "draw.oval",
        &json!({ "x": x, "y": y, "width": w, "height": h, "fill": fill, "stroke": { "color": "#222222", "width": 3 }, "object_drawing": true }),
    )
    .unwrap();
}

fn select(s: &mut Session<AniDoc>, indices: &[usize]) {
    s.execute("select.all", &json!({})).unwrap();
    let keep: Vec<_> = s.doc().editor.state.selection.items.iter().filter(|r| indices.contains(&r.path[0])).cloned().collect();
    s.doc_mut().editor.state.selection = s.doc().editor.state.selection.with_items(keep);
}

fn save(s: &Session<AniDoc>, name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("ANICRAFT_SHOT_DIR")?);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = s.doc();
    let (bytes, _) = anicraft_xfl::write_fla_bytes(&doc.editor.document, &doc.media, &doc.sidecars, &WriteOptions::default()).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    Some(path)
}

fn round_trip(s: &Session<AniDoc>) -> AniDoc {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.fla");
    let doc = s.doc();
    let (bytes, _) = anicraft_xfl::write_fla_bytes(&doc.editor.document, &doc.media, &doc.sidecars, &WriteOptions::default()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    AniDoc::from(Project::open(&path).unwrap())
}

fn objects(doc: &AniDoc) -> Vec<Element> {
    doc.document().scenes[0].layers.iter().flat_map(|l| l.keyframe_at(0).map(|k| k.elements.clone()).unwrap_or_default()).collect()
}

#[test]
fn combined_objects_and_shape_edits_survive_a_save() {
    let mut s = session();
    // Four overlapping pairs side by side: union, intersect, punch, crop.
    for (i, op) in ["union", "intersect", "punch", "crop"].iter().enumerate() {
        let x = 20.0 + 130.0 * f64::from(u8::try_from(i).unwrap());
        rect(&mut s, x, 40.0, 70.0, 70.0, "#e8553d");
        oval(&mut s, x + 35.0, 70.0, 70.0, 70.0, "#2f80ed");
        let n = s.doc().document().scenes[0].layers[0].keyframe_at(0).unwrap().elements.len();
        select(&mut s, &[n - 2, n - 1]);
        s.execute("modify.combine_objects", &json!({ "op": op })).unwrap();
    }
    assert_eq!(s.doc().document().scenes[0].layers[0].keyframe_at(0).unwrap().elements.len(), 4);
    let back = round_trip(&s);
    let shapes: Vec<_> = objects(&back).into_iter().filter_map(|e| if let Element::Shape(sh) = e { Some(sh) } else { None }).collect();
    assert_eq!(shapes.len(), 4);
    assert!(shapes.iter().all(|sh| sh.is_drawing_object && fill_area(&sh.shape) > 100.0));
    save(&s, "shape-ops-combine.fla");
}

#[test]
fn shape_edits_expand_soften_and_lines_to_fills_round_trip() {
    let mut s = session();
    rect(&mut s, 30.0, 40.0, 100.0, 80.0, "#3fb950");
    rect(&mut s, 190.0, 40.0, 100.0, 80.0, "#d29922");
    rect(&mut s, 350.0, 40.0, 100.0, 80.0, "#a371f7");
    select(&mut s, &[0]);
    s.execute("modify.expand_fill", &json!({ "distance": 12 })).unwrap();
    select(&mut s, &[1]);
    s.execute("modify.soften_fill_edges", &json!({ "distance": 14, "steps": 5 })).unwrap();
    select(&mut s, &[2]);
    s.execute("modify.convert_lines_to_fills", &json!({})).unwrap();
    let back = round_trip(&s);
    let areas: Vec<f64> = objects(&back).iter().filter_map(|e| if let Element::Shape(sh) = e { Some(fill_area(&sh.shape)) } else { None }).collect();
    assert_eq!(areas.len(), 3);
    assert!(areas[0] > 8_000.0 + 1_500.0, "{areas:?}");
    assert!(areas[1] > 8_000.0, "{areas:?}");
    save(&s, "shape-ops-edits.fla");
}

#[test]
fn locks_and_transforms_are_saved_in_the_file() {
    let mut s = session();
    rect(&mut s, 40.0, 40.0, 90.0, 60.0, "#e8553d");
    rect(&mut s, 200.0, 40.0, 90.0, 60.0, "#2f80ed");
    select(&mut s, &[0]);
    s.execute("modify.lock", &json!({})).unwrap();
    select(&mut s, &[1]);
    s.execute("modify.transform", &json!({ "op": "rotate_90_cw" })).unwrap();
    let back = round_trip(&s);
    let all = objects(&back);
    assert!(is_locked(&all[0]), "the lock is written as the object's locked attribute");
    assert!(!is_locked(&all[1]));
    let m = all[1].matrix().unwrap().as_coeffs();
    assert!(m[1].abs() > 0.9, "the rotation is kept: {m:?}");
}

#[test]
fn breaking_apart_text_and_bitmaps_gives_shapes() {
    let mut s = session();
    s.execute("draw.rectangle", &json!({ "x": 10, "y": 10, "width": 60, "height": 40, "fill": "#cc3333", "object_drawing": true })).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("modify.convert_to_bitmap", &json!({ "name": "card" })).unwrap();
    let before = objects(s.doc());
    assert!(matches!(before[0], Element::Bitmap(_)));
    s.execute("modify.break_apart", &json!({})).unwrap();
    let after = objects(s.doc());
    let Element::Shape(sh) = &after[0] else { panic!("the bitmap became a shape: {after:?}") };
    assert!(matches!(sh.shape.fill(1), Some(anim_shape::FillStyle::Bitmap(_))));
    // The shape still draws the picture after a save.
    let back = round_trip(&s);
    let Element::Shape(sh) = &objects(&back)[0] else { panic!("shape") };
    assert!(matches!(sh.shape.fill(1), Some(anim_shape::FillStyle::Bitmap(_))));
}

#[test]
fn breaking_apart_text_gives_a_field_per_letter_then_shapes() {
    if anim_text::TextEngine::new().families().is_empty() {
        return;
    }
    let mut s = session();
    s.execute("draw.text", &json!({ "x": 40, "y": 60, "text": "Hi!", "size": 48 })).unwrap();
    save(&s, "shape-ops-text.fla");
    s.execute("select.all", &json!({})).unwrap();
    s.execute("modify.break_apart", &json!({})).unwrap();
    let letters = objects(s.doc());
    let texts: Vec<_> = letters.iter().filter_map(|e| if let Element::Text(t) = e { Some(t) } else { None }).collect();
    assert_eq!(texts.len(), 3, "one field per letter: {letters:?}");
    let plain: Vec<String> = texts.iter().map(|t| t.plain_text()).collect();
    assert_eq!(plain, ["H", "i", "!"]);
    let xs: Vec<f64> = texts.iter().map(|t| t.matrix.as_coeffs()[4]).collect();
    assert!(xs[0] >= 40.0 && xs[0] < xs[1] && xs[1] < xs[2], "letters keep their order and spacing: {xs:?}");
    // The letters are saved as fields, and the next Break Apart turns them into outlines.
    let back = round_trip(&s);
    assert_eq!(objects(&back).iter().filter(|e| matches!(e, Element::Text(_))).count(), 3);
    save(&s, "shape-ops-letters.fla");
    s.execute("select.all", &json!({})).unwrap();
    s.execute("modify.break_apart", &json!({})).unwrap();
    let shapes = objects(s.doc());
    assert_eq!(shapes.len(), 1, "the outlines merge into one shape: {shapes:?}");
    assert!(matches!(&shapes[0], Element::Shape(sh) if !sh.is_drawing_object && fill_area(&sh.shape) > 100.0));
    // A single letter goes straight to an outline.
    let mut single = session();
    single.execute("draw.text", &json!({ "x": 40, "y": 60, "text": "W", "size": 48 })).unwrap();
    single.execute("select.all", &json!({})).unwrap();
    single.execute("modify.break_apart", &json!({})).unwrap();
    assert!(matches!(&objects(single.doc())[0], Element::Shape(_)));
}

#[test]
fn distributing_and_pasting_edit_the_timeline() {
    let mut s = session();
    for i in 0..3 {
        rect(&mut s, 20.0 + 100.0 * f64::from(i), 40.0, 70.0, 70.0, "#e8553d");
    }
    s.execute("select.all", &json!({})).unwrap();
    s.execute("modify.distribute_to_layers", &json!({})).unwrap();
    let back = round_trip(&s);
    assert_eq!(back.document().scenes[0].layers.len(), 4);
    save(&s, "shape-ops-layers.fla");
}
