//! Reads the hand-written fixture in `tests/fixtures/basic` from a folder, from a zip and from its `*.xfl` marker,
//! and checks what lands in the model and what stays in `extra`.
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

use std::io::Write;
use std::path::Path;

use anicraft_xfl::{ReadOptions, decode_bitmap, read, read_bytes, read_with, sound_payload};
use anim_archive::{ArchiveWriter, ZipWriter};
use anim_core::{BlendMode, Rgba8};
use anim_display::Filter;
use anim_scene::{
    ColorEffect, Document, EaseCurve, EaseTarget, Element, InstanceFilter, LabelType, LayerKind, LibraryItem, LoopMode, SoundLoop, SoundSync, SymbolKind,
    TextAlign, TextOrientation, TweenRotate, TweenType,
};
use anim_shape::{FillStyle, StrokeKind, StrokeStyle};
use flate2::Compression;
use flate2::write::ZlibEncoder;

const DOM: &str = include_str!("../fixtures/basic/DOMDocument.xml");
const BALL: &str = include_str!("../fixtures/basic/LIBRARY/Ball.xml");
const WEIRD: &str = include_str!("../fixtures/basic/LIBRARY/shapes/&#060Weird&#062.xml");
const PUBLISH: &str = include_str!("../fixtures/basic/PublishSettings.xml");
const METADATA: &str = include_str!("../fixtures/basic/META-INF/metadata.xml");
const MARKER: &str = include_str!("../fixtures/basic/basic.xfl");

const WEIRD_NAME: &str = "shapes/&#060Weird&#062";

/// A 4 x 2 lossless bitmap: row 0 opaque red, green, blue, white; row 1 half transparent grey, then three clear pixels.
fn bitmap_dat() -> Vec<u8> {
    let argb: [u8; 32] = [255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 128, 100, 100, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
    let mut out = vec![0x03, 0x05];
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    for v in [0u32, 80, 0, 40] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[0, 1]);
    let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&argb).unwrap();
    let packed = z.finish().unwrap();
    out.extend_from_slice(&u16::try_from(packed.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&packed);
    out.extend_from_slice(&[0, 0]);
    out
}

/// Eight bytes of sound followed by five bytes of preview cache.
fn sound_dat() -> Vec<u8> {
    vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 9, 9, 9, 9]
}

fn entries() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("DOMDocument.xml", DOM.as_bytes().to_vec()),
        ("LIBRARY/Ball.xml", BALL.as_bytes().to_vec()),
        ("LIBRARY/shapes/&#060Weird&#062.xml", WEIRD.as_bytes().to_vec()),
        ("PublishSettings.xml", PUBLISH.as_bytes().to_vec()),
        ("META-INF/metadata.xml", METADATA.as_bytes().to_vec()),
        ("basic.xfl", MARKER.as_bytes().to_vec()),
        ("bin/M 1 1700000001.dat", bitmap_dat()),
        ("bin/M 2 1700000002.dat", sound_dat()),
    ]
}

fn zip_bytes() -> Vec<u8> {
    let mut writer = ZipWriter::in_memory();
    for (name, data) in entries() {
        writer.add_file(name, &data).unwrap();
    }
    writer.finish().unwrap()
}

fn write_folder(root: &Path) {
    for (name, data) in entries() {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }
}

fn load() -> anicraft_xfl::Loaded {
    read_bytes(zip_bytes(), &ReadOptions::default()).unwrap()
}

#[test]
fn document_settings_and_extras() {
    let loaded = load();
    let doc = &loaded.document;
    assert_eq!((doc.width, doc.height), (640.0, 360.0));
    assert_eq!(doc.frame_rate, 12.0);
    assert_eq!(doc.background_color, Rgba8::opaque(0x11, 0x22, 0x33));
    assert_eq!(doc.current_timeline, 1);
    assert_eq!(doc.scenes.len(), 2);
    assert_eq!(doc.extra.attr("fixtureAttribute"), Some("kept"));
    assert_eq!(doc.extra.attr("xflVersion"), Some("2.9"));
    assert!(doc.extra.children.iter().any(|c| c == "<scripts/>"));
    assert!(doc.extra.children.iter().any(|c| c == "<PrinterSettings/>"));
    assert!(doc.extra.children.iter().any(|c| c.starts_with("<DOMCompiledClipItem")));
    assert!(doc.publish_settings_xml.as_deref().is_some_and(|p| p.contains("flash_profiles")));
    assert_eq!(loaded.unknowns.children.get("media/DOMCompiledClipItem"), Some(&1));
    let names: Vec<&str> = loaded.sidecars.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"META-INF/metadata.xml") && names.contains(&"basic.xfl"), "{names:?}");
    assert!(loaded.warnings.iter().any(|w| w.message.contains("Missing.xml")), "{:?}", loaded.warnings);
}

#[test]
fn library_items_and_media() {
    let loaded = load();
    let lib = &loaded.document.library;
    let order: Vec<(&str, &str)> = lib.items.iter().map(|i| (i.kind_name(), i.name())).collect();
    assert_eq!(
        order,
        [("folder", "shapes"), ("bitmap", "shapes/photo"), ("sound", "click.wav"), ("font", "Embedded Arial"), ("symbol", "Ball"), ("symbol", WEIRD_NAME),]
    );
    let bitmap = lib.find_bitmap("shapes/photo").unwrap();
    assert_eq!(bitmap.size, Some([4.0, 2.0]));
    assert!(bitmap.smoothing);
    assert_eq!(bitmap.quality, Some(50));
    assert_eq!(bitmap.extra.attr("sourceLastImported"), Some("1700000001"));
    let image = decode_bitmap(&loaded.media, bitmap.media.unwrap()).unwrap();
    assert_eq!((image.width, image.height), (4, 2));
    assert_eq!(&image.premultiplied_rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&image.premultiplied_rgba[16..20], &[50, 50, 50, 128]);
    assert_eq!(&image.premultiplied_rgba[20..24], &[0, 0, 0, 0]);
    let sound = lib.find_sound("click.wav").unwrap();
    assert_eq!(sound.format.as_deref(), Some("22kHz 16bit Mono"));
    assert_eq!(sound.sample_count, Some(4));
    assert_eq!(sound_payload(&loaded.media, sound), Some(&[1u8, 2, 3, 4, 5, 6, 7, 8][..]));
    let Some(LibraryItem::Font(font)) = lib.find("Embedded Arial") else {
        panic!("font item");
    };
    assert_eq!(font.font.as_deref(), Some("ArialMT"));
    assert_eq!(font.extra.attr("embedRanges"), Some("1|2"));
    let Some(LibraryItem::Folder(folder)) = lib.find("shapes") else {
        panic!("folder item");
    };
    assert!(folder.is_expanded);
}

#[test]
fn symbols_shapes_and_styles() {
    let loaded = load();
    let lib = &loaded.document.library;
    let ball = lib.find_symbol("Ball").unwrap();
    assert_eq!(ball.kind, SymbolKind::Graphic);
    assert_eq!(ball.item_id, "f0000001-00000006");
    assert_eq!(ball.last_modified, Some(1_700_000_010));
    assert_eq!(ball.href.as_deref(), Some("Ball.xml"));
    assert!(ball.include_attrs.contains(&("loadImmediate".to_owned(), "false".to_owned())));
    assert_eq!(ball.extra.attr("extraSymbolAttribute"), Some("yes"));
    let linkage = ball.linkage.as_ref().unwrap();
    assert!(linkage.export_for_actionscript);
    assert_eq!(linkage.class_name.as_deref(), Some("Ball"));
    assert_eq!(linkage.base_class.as_deref(), Some("flash.display.Sprite"));
    let frame = &ball.timeline.layers[0].frames[0];
    assert_eq!(frame.duration, 8);
    let Element::Shape(shape) = &frame.elements[0] else {
        panic!("shape");
    };
    assert_eq!(shape.shape.fills.len(), 2);
    let FillStyle::RadialGradient(g) = &shape.shape.fills[0] else {
        panic!("radial gradient");
    };
    assert_eq!(g.spread, anim_shape::Spread::Reflect);
    assert_eq!(g.interpolation, anim_shape::Interpolation::LinearRgb);
    assert!((g.focal - 0.4).abs() < 1e-9);
    assert_eq!(g.stops.len(), 2);
    assert_eq!(g.stops[1].color, Rgba8::new(0, 0, 255, 128));
    let FillStyle::Bitmap(b) = &shape.shape.fills[1] else {
        panic!("bitmap fill");
    };
    assert_eq!((b.path.as_str(), b.clipped), ("shapes/photo", true));
    assert_eq!(shape.shape.edges.len(), 2);
    assert_eq!(shape.shape.edges[0].original_edges_text(), Some("!0 0[100 0 100 100!100 100|0 100!0 100|0 0"));
    assert!(!shape.shape.edges[0].segments.is_empty());
    assert_eq!(shape.shape.edges[1].fill_style0, 2);

    let weird = lib.find_symbol(WEIRD_NAME).unwrap();
    assert_eq!(weird.kind, SymbolKind::Button);
    assert_eq!(weird.href.as_deref(), Some("shapes/&#060Weird&#062.xml"));
    let grid = weird.scale9_grid.unwrap();
    assert_eq!((grid.x0, grid.y0, grid.x1, grid.y1), (-10.0, -5.0, 10.5, 5.25));
    assert_eq!(weird.timeline.layers[0].frames.len(), 4);
    assert_eq!(weird.timeline.layer_depth_enabled, Some(false));
    assert!(!weird.timeline.layers[0].autoname);
}

#[test]
fn scene_layers_and_kinds() {
    let loaded = load();
    let t = &loaded.document.scenes[0];
    assert_eq!(t.name, "Scene 1");
    assert_eq!(t.current_frame, 3);
    assert_eq!(t.layer_depth_enabled, Some(true));
    let kinds: Vec<LayerKind> = t.layers.iter().map(|l| l.kind).collect();
    assert_eq!(
        kinds,
        [LayerKind::Folder, LayerKind::Mask, LayerKind::Normal, LayerKind::Guide, LayerKind::Normal, LayerKind::Normal, LayerKind::Normal, LayerKind::Normal]
    );
    assert_eq!(t.layers[1].parent_layer_index, Some(0));
    assert_eq!(t.layers[2].parent_layer_index, Some(1));
    assert_eq!(t.resolved_kind(2), Some(LayerKind::Masked));
    assert!(t.layers[1].locked);
    assert!(!t.layers[2].visible && t.layers[2].outline);
    assert!(!t.layers[0].autoname && t.layers[4].autoname);
    assert_eq!(t.layers[4].color, Rgba8::opaque(0, 255, 255));
    assert_eq!(t.layers[7].extra.attr("customLayerAttribute"), Some("7"));
    assert_eq!(t.layers[6].extra.attr("current"), Some("true"));
    assert_eq!(t.frame_count(), 24);
}

#[test]
fn mask_shape_bitmap_instance_and_drawing_object() {
    let loaded = load();
    let t = &loaded.document.scenes[0];
    let Element::Shape(s) = &t.layers[1].frames[0].elements[0] else {
        panic!("mask shape");
    };
    assert!(s.is_drawing_object);
    assert_eq!(s.matrix, anim_core::Affine::new([2.0, 0.0, 0.0, 2.0, 10.0, 20.0]));
    assert_eq!(s.transformation_point, Some(anim_core::Point::new(5.0, 5.0)));
    assert_eq!(s.shape.fills[0], FillStyle::Solid(Rgba8::new(0, 255, 0, 128)));
    let Element::Bitmap(b) = &t.layers[2].frames[0].elements[0] else {
        panic!("bitmap instance");
    };
    assert_eq!(b.library_item_name, "shapes/photo");
    assert_eq!(b.matrix, anim_core::Affine::new([1.0, 0.5, -0.5, 1.0, 1.5, -2.25]));
    assert_eq!(b.extra.attr("extraBitmapAttribute"), Some("x"));
}

#[test]
fn tweens_instances_colours_and_filters() {
    let loaded = load();
    let layer = &loaded.document.scenes[0].layers[4];
    let f = &layer.frames[0];
    assert_eq!(f.tween_type, TweenType::Classic);
    assert_eq!(f.motion_tween_rotate, TweenRotate::Clockwise);
    assert_eq!(f.motion_tween_rotate_times, Some(2.0));
    assert!(!f.motion_tween_scale && f.motion_tween_snap && f.motion_tween_orient_to_path && f.motion_tween_sync);
    assert_eq!(f.acceleration, -30);
    assert_eq!(f.ease_method_name.as_deref(), Some("cubicIn"));
    assert_eq!(f.key_mode, 22017);
    assert_eq!((f.label.as_deref(), f.label_type), (Some("start"), Some(LabelType::Anchor)));
    assert_eq!(f.eases.len(), 2);
    assert_eq!(f.ease_for(EaseTarget::Position), Some(&EaseCurve::Named("cubicIn".into())));
    let Some(EaseCurve::Custom(points)) = f.ease_for(EaseTarget::Rotation) else {
        panic!("custom ease");
    };
    assert_eq!(points, &vec![[0.0, 0.0], [0.25, 0.1], [0.75, 0.9], [1.0, 1.0]]);
    assert!(f.extra.is_empty(), "{:?}", f.extra);

    let i = f.elements[0].as_symbol_instance().unwrap();
    assert_eq!(i.library_item_name, "Ball");
    assert_eq!(i.symbol_type, Some(SymbolKind::Graphic));
    assert_eq!(i.loop_mode, Some(LoopMode::PlayOnce));
    assert_eq!((i.first_frame, i.last_frame), (2, Some(5)));
    assert_eq!(i.name.as_deref(), Some("ball1"));
    assert_eq!(i.blend_mode, BlendMode::Screen);
    assert!(!i.visible && i.cache_as_bitmap);
    assert_eq!(i.transformation_point, anim_core::Point::new(0.5, 0.5));
    assert_eq!(i.extra.attr("centerPoint3DX"), Some("1.5"));
    assert!(matches!(i.color_effect, ColorEffect::Tint { color, amount } if color == Rgba8::opaque(255, 136, 0) && (amount - 0.4).abs() < 1e-6));
    assert_eq!(i.filters.len(), 4);
    assert!(matches!(&i.filters[0], InstanceFilter::Mapped { filter: Filter::Blur { quality: 2, x, y }, .. } if *x == 8.0 && *y == 4.0));
    let InstanceFilter::Mapped { filter: Filter::DropShadow { color, angle_deg, distance, strength, inner, hide_object, knockout, .. }, extra } = &i.filters[1]
    else {
        panic!("shadow");
    };
    assert_eq!(color.a, 191);
    assert_eq!((*angle_deg, *distance, *strength), (90.0, 6.0, 1.5));
    assert!(*inner && *hide_object && !*knockout);
    assert_eq!(extra.attr("quality"), Some("1"));
    assert!(matches!(&i.filters[2], InstanceFilter::Mapped { filter: Filter::Glow { knockout: true, .. }, .. }));
    assert!(matches!(&i.filters[3], InstanceFilter::Raw(r) if r.tag == "BevelFilter"));

    let second = layer.frames[1].elements[0].as_symbol_instance().unwrap();
    let ColorEffect::Advanced(t) = second.color_effect else {
        panic!("advanced colour");
    };
    assert!((t.mul[3] - 0.25).abs() < 1e-6 && (t.add[0] - 0.2).abs() < 1e-6 && (t.mul[0] - 1.0).abs() < 1e-6);
    assert_eq!(second.symbol_type, Some(SymbolKind::Graphic));
}

#[test]
fn shape_tween_keeps_the_morph_shape_verbatim() {
    let loaded = load();
    let f = &loaded.document.scenes[0].layers[5].frames[0];
    assert_eq!(f.tween_type, TweenType::Shape);
    let xml = f.morph_shape_xml.as_deref().unwrap();
    assert!(xml.starts_with("<MorphShape>") && xml.contains("<morphHintsList/>") && xml.contains(r#"startPointA="1, 2""#), "{xml}");
    assert_eq!(f.key_mode, 17922);
}

#[test]
fn text_is_unicode_with_runs_and_extras() {
    let loaded = load();
    let f = &loaded.document.scenes[0].layers[6].frames[0];
    assert_eq!(f.elements.len(), 3);
    let Element::Text(stat) = &f.elements[0] else {
        panic!("static text");
    };
    assert_eq!(stat.text_runs.len(), 2);
    assert_eq!(stat.text_runs[0].characters, "שלום עולם hello مرحبا");
    assert_eq!(stat.text_runs[1].characters, "second\rline");
    assert_eq!(stat.plain_text(), "שלום עולם hello مرحباsecond\rline");
    let run = &stat.text_runs[0];
    assert!(run.bold && run.italic);
    assert_eq!((run.alignment, run.size, run.letter_spacing), (TextAlign::Right, 24.0, 2.0));
    assert_eq!(run.fill_color, Rgba8::opaque(0, 0x66, 0xCC));
    assert_eq!(run.extra.attr("aliasText"), Some("false"));
    assert!((stat.text_runs[1].alpha - 0.5).abs() < 1e-6);
    assert!(stat.auto_expand);
    assert_eq!(stat.extra.attr("isSelectable"), Some("false"));
    assert_eq!(stat.matrix, anim_core::Affine::translate((20.0, 300.0)));
    let Element::Text(dynamic) = &f.elements[1] else {
        panic!("dynamic text");
    };
    assert_eq!(dynamic.kind, anim_scene::TextKind::Dynamic);
    assert_eq!(dynamic.instance_name.as_deref(), Some("score"));
    assert_eq!(dynamic.extra.attr("lineType"), Some("multiline"));
    assert_eq!(dynamic.extra.attr("name"), Some("score"));
    let Element::Text(input) = &f.elements[2] else {
        panic!("input text");
    };
    assert_eq!(input.kind, anim_scene::TextKind::Input);
    assert_eq!(input.orientation, TextOrientation::VerticalRightToLeft);
    assert_eq!(input.extra.attr("border"), Some("true"));
}

#[test]
fn sounds_scripts_labels_groups_and_strokes() {
    let loaded = load();
    let layer = &loaded.document.scenes[0].layers[7];
    let f = &layer.frames[0];
    let s = f.sound.as_ref().unwrap();
    assert_eq!(s.name, "click.wav");
    assert_eq!(s.sync, SoundSync::Start);
    assert_eq!(s.loop_mode, SoundLoop::Repeat(3));
    assert_eq!(s.effect.as_deref(), Some("fade in"));
    assert_eq!(s.envelope.len(), 2);
    assert_eq!(s.envelope[1].level0, 32768);
    assert_eq!(s.extra.attr("inPoint44"), Some("10"));
    assert_eq!(s.extra.attr("soundZoomLevel"), Some("2"));
    assert_eq!(f.actionscript.as_deref(), Some("stop();\ntrace(\"a < b && c\");"));
    assert!(f.extra.is_empty(), "{:?}", f.extra);

    let g = &layer.frames[1];
    assert_eq!((g.label.as_deref(), g.label_type), (Some("done"), Some(LabelType::Comment)));
    let Element::Group(group) = &g.elements[0] else {
        panic!("group");
    };
    assert_eq!(group.matrix, anim_core::Affine::translate((7.0, 8.0)));
    assert_eq!(group.transformation_point, Some(anim_core::Point::new(1.0, 2.0)));
    assert_eq!(group.extra.attr("selected"), Some("true"));
    assert_eq!(group.members.len(), 2);
    let Element::Shape(shape) = &group.members[0] else {
        panic!("stroke shape");
    };
    let StrokeStyle::Solid(solid) = &shape.shape.strokes[0] else {
        panic!("solid stroke");
    };
    assert_eq!(solid.weight, 3.0);
    assert_eq!(solid.caps, anim_shape::Cap::Square);
    assert_eq!(solid.joints, anim_shape::Join::Bevel);
    assert_eq!(solid.miter_limit, 8.0);
    assert_eq!(solid.scale_mode, anim_shape::StrokeScale::None);
    assert!(solid.pixel_hinting);
    let StrokeStyle::Other(other) = &shape.shape.strokes[1] else {
        panic!("dotted stroke");
    };
    assert_eq!(other.kind, StrokeKind::Dotted);
    assert_eq!(other.attrs, vec![("dotSpace".to_owned(), "3".to_owned()), ("sharpness".to_owned(), "4".to_owned())]);
    let Element::Group(inner) = &group.members[1] else {
        panic!("inner group");
    };
    assert!(matches!(&inner.members[0], Element::Unknown(r) if r.tag == "DOMVideoInstance" && r.xml.contains("clip.flv")));
}

#[test]
fn scene_two_references_the_escaped_symbol_name() {
    let loaded = load();
    let doc = &loaded.document;
    let i = doc.scenes[1].layers[0].frames[0].elements[0].as_symbol_instance().unwrap();
    assert_eq!(i.library_item_name, WEIRD_NAME);
    assert_eq!(i.matrix, anim_core::Affine::IDENTITY);
    assert!(doc.library.find_symbol(&i.library_item_name).is_some());
    assert!(doc.validate().is_empty(), "{:?}", doc.validate());
}

#[test]
fn folder_zip_and_marker_read_identically() {
    let zipped = load();
    let dir = tempfile::tempdir().unwrap();
    write_folder(dir.path());
    let folder = read_with(dir.path(), &ReadOptions::default()).unwrap();
    assert!(folder.source.folder);
    assert_eq!(folder.document, zipped.document);
    assert_eq!(folder.media, zipped.media);
    let marker = read_with(dir.path().join("basic.xfl"), &ReadOptions::default()).unwrap();
    assert_eq!(marker.document, zipped.document);
    let file = dir.path().join("fixture.fla");
    std::fs::write(&file, zip_bytes()).unwrap();
    let (doc, media, warnings) = read(&file).unwrap();
    assert_eq!(doc, zipped.document);
    assert_eq!(media.len(), zipped.media.len());
    assert_eq!(warnings, zipped.warnings);
}

#[test]
fn skipping_media_leaves_ids_empty() {
    let loaded = read_bytes(zip_bytes(), &ReadOptions { load_media: false, ..ReadOptions::default() }).unwrap();
    assert!(loaded.media.is_empty());
    assert!(loaded.document.library.bitmaps().all(|b| b.media.is_none()));
}

#[test]
fn the_document_round_trips_through_json() {
    let loaded = load();
    let json = serde_json::to_string(&loaded.document).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, loaded.document);
}

#[test]
fn the_summary_counts_and_serialises() {
    let loaded = load();
    let summary = loaded.summary(3);
    assert_eq!((summary.scenes, summary.symbols, summary.bitmaps, summary.sounds, summary.fonts, summary.folders), (2, 2, 1, 1, 1, 1));
    assert_eq!(summary.layers, 8 + 1 + 1 + 1);
    assert_eq!((summary.width, summary.frame_rate), (640.0, 12.0));
    assert_eq!(summary.media_blobs, 2);
    assert_eq!(summary.media_bytes, u64::try_from(bitmap_dat().len() + sound_dat().len()).unwrap());
    assert_eq!(summary.top_unknowns.len(), 3);
    assert!(summary.top_unknowns.windows(2).all(|w| w[0].1 >= w[1].1));
    let json = serde_json::to_string(&summary).unwrap();
    assert_eq!(serde_json::from_str::<anicraft_xfl::LoadSummary>(&json).unwrap(), summary);
}

#[test]
fn missing_document_and_wrong_root_are_errors() {
    let mut writer = ZipWriter::in_memory();
    writer.add_file("something.txt", b"x").unwrap();
    assert!(matches!(read_bytes(writer.finish().unwrap(), &ReadOptions::default()), Err(anicraft_xfl::XflError::MissingDocument)));
    let mut writer = ZipWriter::in_memory();
    writer.add_file("DOMDocument.xml", b"<html/>").unwrap();
    assert!(matches!(read_bytes(writer.finish().unwrap(), &ReadOptions::default()), Err(anicraft_xfl::XflError::NotXfl(_))));
    let mut writer = ZipWriter::in_memory();
    writer.add_file("DOMDocument.xml", b"<DOMDocument").unwrap();
    assert!(matches!(read_bytes(writer.finish().unwrap(), &ReadOptions::default()), Err(anicraft_xfl::XflError::Xml { .. })));
}
