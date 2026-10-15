//! Reads exported SWFs back with Ruffle's `swf` crate (an independent parser) and checks what was written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::missing_const_for_fn,
    clippy::panic,
    clippy::too_many_lines
)]

use std::io::Read;

use anicraft_swf::export::{DisplayListExporter, ExportNote, ExportOptions, export_display_lists, export_still};
use anicraft_swf::movie::{ACTION_STOP, Compression, Frame, FrameItem, Movie, MovieSettings};
use anicraft_swf::place::Placement;
use anicraft_swf::shape::{PathStyle, ShapeBuilder};
use anicraft_swf::sound::{Mp3Stream, SoundInfo, split_mp3_stream};
use anicraft_swf::style::FillStyle;
use anicraft_swf::types::{CharacterId, Matrix};
use anim_core::kurbo::{self, BezPath, Shape};
use anim_core::{Affine, BlendMode, ColorTransform, Point, Rgba8, Size};
use anim_display::{
    Bitmap, Cap, DisplayList, Fill, FillRule, Filter, Gradient, GradientStop, Group, Image, ImageStore, Interpolation, Join, Mesh, Node, Paint, Spread, Stroke,
    StrokeScale, StrokeStyle,
};

fn rect_path(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    kurbo::Rect::new(x, y, x + w, y + h).to_path(0.1)
}

fn solid_fill(path: BezPath, color: Rgba8, transform: Affine) -> Node {
    Node::Fill(Fill { transform, path, rule: FillRule::NonZero, paint: Paint::Solid(color) })
}

fn list(nodes: Vec<Node>) -> DisplayList {
    DisplayList { size: Size::new(550.0, 400.0), background: Some(Rgba8::opaque(0x12, 0x34, 0x56)), nodes }
}

/// Decompresses and parses with Ruffle; panics on anything Ruffle rejects.
fn parse(bytes: &[u8]) -> swf::SwfBuf {
    swf::decompress_swf(bytes).expect("swf header and body decompress")
}

fn tag_names(tags: &[swf::Tag]) -> Vec<&'static str> {
    tags.iter()
        .map(|t| match t {
            swf::Tag::FileAttributes(_) => "FileAttributes",
            swf::Tag::SetBackgroundColor(_) => "SetBackgroundColor",
            swf::Tag::DefineShape(_) => "DefineShape",
            swf::Tag::DefineSprite(_) => "DefineSprite",
            swf::Tag::DefineBitsLossless(_) => "DefineBitsLossless",
            swf::Tag::PlaceObject(_) => "PlaceObject",
            swf::Tag::RemoveObject(_) => "RemoveObject",
            swf::Tag::ShowFrame => "ShowFrame",
            swf::Tag::DoAction(_) => "DoAction",
            swf::Tag::DefineSound(_) => "DefineSound",
            swf::Tag::StartSound(_) => "StartSound",
            swf::Tag::SoundStreamHead2(_) => "SoundStreamHead2",
            swf::Tag::SoundStreamBlock(_) => "SoundStreamBlock",
            swf::Tag::FrameLabel(_) => "FrameLabel",
            swf::Tag::DefineSceneAndFrameLabelData(_) => "DefineSceneAndFrameLabelData",
            swf::Tag::Metadata(_) => "Metadata",
            swf::Tag::ExportAssets(_) => "ExportAssets",
            _ => "Other",
        })
        .collect()
}

fn count(tags: &[swf::Tag], name: &str) -> usize {
    tag_names(tags).iter().filter(|n| **n == name).count()
}

/// The anchor points of every contour of a shape, in twips.
fn contours(shape: &swf::Shape) -> Vec<Vec<(i32, i32)>> {
    let mut out: Vec<Vec<(i32, i32)>> = Vec::new();
    let (mut x, mut y) = (0, 0);
    for record in &shape.shape {
        match record {
            swf::ShapeRecord::StyleChange(change) => {
                if let Some(p) = change.move_to {
                    (x, y) = (p.x.get(), p.y.get());
                    out.push(vec![(x, y)]);
                }
            }
            swf::ShapeRecord::StraightEdge { delta } => {
                x += delta.dx.get();
                y += delta.dy.get();
                out.last_mut().unwrap().push((x, y));
            }
            swf::ShapeRecord::CurvedEdge { control_delta, anchor_delta } => {
                x += control_delta.dx.get() + anchor_delta.dx.get();
                y += control_delta.dy.get() + anchor_delta.dy.get();
                out.last_mut().unwrap().push((x, y));
            }
        }
    }
    out
}

fn first_shape<'a>(tags: &'a [swf::Tag<'a>]) -> &'a swf::Shape {
    tags.iter().find_map(|t| if let swf::Tag::DefineShape(s) = t { Some(&**s) } else { None }).expect("a DefineShape")
}

fn places<'a>(tags: &'a [swf::Tag<'a>]) -> Vec<&'a swf::PlaceObject<'a>> {
    tags.iter().filter_map(|t| if let swf::Tag::PlaceObject(p) = t { Some(&**p) } else { None }).collect()
}

fn gradient(spread: Spread, interpolation: Interpolation, focal: f64) -> Gradient {
    Gradient {
        transform: Affine::new([0.5, 0.0, 0.0, 0.25, 100.0, 50.0]),
        stops: vec![
            GradientStop { offset: 0.0, color: Rgba8::new(255, 0, 0, 255) },
            GradientStop { offset: 0.5, color: Rgba8::new(0, 255, 0, 128) },
            GradientStop { offset: 1.0, color: Rgba8::new(0, 0, 255, 0) },
        ],
        spread,
        interpolation,
        focal,
    }
}

#[test]
fn single_rectangle_round_trips_header_and_geometry() {
    let l = list(vec![solid_fill(rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::opaque(255, 0, 0), Affine::translate((30.0, 40.0)))]);
    let out = export_still(&l, &ImageStore::new()).unwrap();
    assert_eq!(&out.swf[..3], b"CWS");
    let buf = parse(&out.swf);
    let h = &buf.header;
    assert_eq!(h.version(), 19);
    assert_eq!(h.compression(), swf::Compression::Zlib);
    assert_eq!((h.stage_size().x_max.get(), h.stage_size().y_max.get()), (11000, 8000));
    assert_eq!(h.frame_rate().to_f32(), 24.0);
    assert_eq!(h.num_frames(), 1);
    let bg = h.background_color().unwrap();
    assert_eq!((bg.r, bg.g, bg.b), (0x12, 0x34, 0x56));
    assert!(!h.is_action_script_3());

    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(tag_names(&parsed.tags), vec!["FileAttributes", "SetBackgroundColor", "DefineShape", "PlaceObject", "ShowFrame"]);
    let shape = first_shape(&parsed.tags);
    assert_eq!(shape.version, 4);
    assert_eq!(contours(shape), vec![vec![(0, 0), (200, 0), (200, 200), (0, 200), (0, 0)]]);
    assert_eq!((shape.shape_bounds.x_min.get(), shape.shape_bounds.x_max.get(), shape.shape_bounds.y_max.get()), (0, 200, 200));
    assert!(shape.flags.contains(swf::ShapeFlag::NON_ZERO_WINDING_RULE));
    assert_eq!(shape.styles.fill_styles, vec![swf::FillStyle::Color(swf::Color { r: 255, g: 0, b: 0, a: 255 })]);
    let place = places(&parsed.tags)[0];
    assert_eq!(place.action, swf::PlaceObjectAction::Place(shape.id));
    assert_eq!(place.depth, 1);
    let m = place.matrix.unwrap();
    assert_eq!((m.tx.get(), m.ty.get()), (600, 800));
    assert!(m.a.is_one() && m.d.is_one() && m.b.is_zero() && m.c.is_zero());
    assert!(place.color_transform.is_none() && place.filters.is_none() && place.blend_mode.is_none());
    assert_eq!(out.report.frames, 1);
}

#[test]
fn even_odd_and_hole_paths() {
    let mut path = rect_path(0.0, 0.0, 100.0, 100.0);
    // a counter-clockwise hole
    let mut hole = BezPath::new();
    hole.move_to((25.0, 25.0));
    hole.line_to((25.0, 75.0));
    hole.line_to((75.0, 75.0));
    hole.line_to((75.0, 25.0));
    hole.close_path();
    path.extend(hole);
    let l = list(vec![Node::Fill(Fill { transform: Affine::IDENTITY, path, rule: FillRule::EvenOdd, paint: Paint::Solid(Rgba8::BLACK) })]);
    let out = export_still(&l, &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let shape = first_shape(&parsed.tags);
    assert!(!shape.flags.contains(swf::ShapeFlag::NON_ZERO_WINDING_RULE));
    let c = contours(shape);
    assert_eq!(c.len(), 2);
    assert_eq!(c[1][0], (500, 500));
}

#[test]
fn gradients_map_to_swf_gradient_fills() {
    let nodes = vec![
        Node::Fill(Fill {
            transform: Affine::IDENTITY,
            path: rect_path(0.0, 0.0, 50.0, 50.0),
            rule: FillRule::NonZero,
            paint: Paint::Linear(gradient(Spread::Reflect, Interpolation::LinearRgb, 0.0)),
        }),
        Node::Fill(Fill {
            transform: Affine::IDENTITY,
            path: rect_path(60.0, 0.0, 50.0, 50.0),
            rule: FillRule::NonZero,
            paint: Paint::Radial(gradient(Spread::Repeat, Interpolation::Rgb, 0.0)),
        }),
        Node::Fill(Fill {
            transform: Affine::IDENTITY,
            path: rect_path(120.0, 0.0, 50.0, 50.0),
            rule: FillRule::NonZero,
            paint: Paint::Radial(gradient(Spread::Pad, Interpolation::Rgb, 0.5)),
        }),
    ];
    let out = export_still(&list(nodes), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let shapes: Vec<_> = parsed.tags.iter().filter_map(|t| if let swf::Tag::DefineShape(s) = t { Some(s) } else { None }).collect();
    assert_eq!(shapes.len(), 3);

    let swf::FillStyle::LinearGradient(g) = &shapes[0].styles.fill_styles[0] else { panic!("linear") };
    assert_eq!(g.spread, swf::GradientSpread::Reflect);
    assert_eq!(g.interpolation, swf::GradientInterpolation::LinearRgb);
    assert_eq!(g.records.iter().map(|r| r.ratio).collect::<Vec<_>>(), vec![0, 128, 255]);
    assert_eq!(g.records[1].color, swf::Color { r: 0, g: 255, b: 0, a: 128 });
    // gradient space is in twips: the linear part is unchanged, the translation goes from px to twips
    assert!((g.matrix.a.to_f64() - 0.5).abs() < 1e-4 && (g.matrix.d.to_f64() - 0.25).abs() < 1e-4);
    assert_eq!((g.matrix.tx.get(), g.matrix.ty.get()), (2000, 1000));

    let swf::FillStyle::RadialGradient(g) = &shapes[1].styles.fill_styles[0] else { panic!("radial") };
    assert_eq!(g.spread, swf::GradientSpread::Repeat);

    let swf::FillStyle::FocalGradient { gradient: g, focal_point } = &shapes[2].styles.fill_styles[0] else { panic!("focal") };
    assert_eq!(g.spread, swf::GradientSpread::Pad);
    assert!((focal_point.to_f64() - 0.5).abs() < 1e-3);
}

#[test]
fn big_gradients_are_reduced_and_noted() {
    let stops = (0..30_u8).map(|i| GradientStop { offset: f32::from(i) / 29.0, color: Rgba8::opaque(i.saturating_mul(8), 0, 0) }).collect();
    let g = Gradient { transform: Affine::IDENTITY, stops, spread: Spread::Pad, interpolation: Interpolation::Rgb, focal: 0.0 };
    let l = list(vec![Node::Fill(Fill { transform: Affine::IDENTITY, path: rect_path(0.0, 0.0, 5.0, 5.0), rule: FillRule::NonZero, paint: Paint::Linear(g) })]);
    let out = export_still(&l, &ImageStore::new()).unwrap();
    assert!(out.report.notes.contains(&ExportNote::GradientStopsReduced));
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let swf::FillStyle::LinearGradient(g) = &first_shape(&parsed.tags).styles.fill_styles[0] else { panic!() };
    assert_eq!(g.records.len(), 15);
}

#[test]
fn strokes_carry_linestyle2_properties() {
    let mut path = BezPath::new();
    path.move_to((10.0, 10.0));
    path.line_to((90.0, 10.0));
    path.line_to((90.0, 60.0));
    let stroke = |style: StrokeStyle, paint: Paint| Node::Stroke(Stroke { transform: Affine::IDENTITY, path: path.clone(), style, paint });
    let nodes = vec![
        stroke(
            StrokeStyle { width: 3.0, cap: Cap::Square, join: Join::Miter, miter_limit: 4.0, scale: StrokeScale::None, dashes: vec![] },
            Paint::Solid(Rgba8::new(1, 2, 3, 200)),
        ),
        stroke(
            StrokeStyle { width: 0.0, cap: Cap::None, join: Join::Bevel, miter_limit: 3.0, scale: StrokeScale::Horizontal, dashes: vec![] },
            Paint::Linear(gradient(Spread::Pad, Interpolation::Rgb, 0.0)),
        ),
    ];
    let out = export_still(&list(nodes), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let shapes: Vec<_> = parsed.tags.iter().filter_map(|t| if let swf::Tag::DefineShape(s) = t { Some(s) } else { None }).collect();
    assert!(shapes[0].flags.contains(swf::ShapeFlag::HAS_NON_SCALING_STROKES));

    let line = &shapes[0].styles.line_styles[0];
    assert_eq!(line.width().get(), 60);
    assert_eq!(line.start_cap(), swf::LineCapStyle::Square);
    assert_eq!(line.end_cap(), swf::LineCapStyle::Square);
    assert!(matches!(line.join_style(), swf::LineJoinStyle::Miter(limit) if (limit.to_f64() - 4.0).abs() < 1e-3));
    assert!(!line.allow_scale_x() && !line.allow_scale_y());
    assert_eq!(*line.fill_style(), swf::FillStyle::Color(swf::Color { r: 1, g: 2, b: 3, a: 200 }));
    // the stroked path is open and starts at the pen position
    assert_eq!(contours(shapes[0]), vec![vec![(200, 200), (1800, 200), (1800, 1200)]]);

    let line = &shapes[1].styles.line_styles[0];
    assert_eq!(line.width().get(), 0);
    assert_eq!(line.start_cap(), swf::LineCapStyle::None);
    assert_eq!(line.join_style(), swf::LineJoinStyle::Bevel);
    assert!(line.allow_scale_x() && !line.allow_scale_y());
    assert!(matches!(line.fill_style(), swf::FillStyle::LinearGradient(_)));
}

#[test]
fn dashed_strokes_become_separate_dashes() {
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    path.line_to((100.0, 0.0));
    let l = list(vec![Node::Stroke(Stroke {
        transform: Affine::IDENTITY,
        path,
        style: StrokeStyle { width: 1.0, cap: Cap::None, join: Join::Round, miter_limit: 3.0, scale: StrokeScale::Normal, dashes: vec![10.0, 10.0] },
        paint: Paint::Solid(Rgba8::BLACK),
    })]);
    let out = export_still(&l, &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let mut c = contours(first_shape(&parsed.tags));
    // the dash iterator may emit the pieces in any order
    c.sort();
    assert_eq!(c.len(), 5);
    assert_eq!(c[0], vec![(0, 0), (200, 0)]);
    assert_eq!(c[1], vec![(400, 0), (600, 0)]);
    assert_eq!(c[4], vec![(1600, 0), (1800, 0)]);
}

#[test]
fn bitmaps_become_lossless2_characters_with_bitmap_fills() {
    let mut images = ImageStore::new();
    let id = images.insert(Image::new(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 128, 0, 0, 0, 0]).unwrap());
    let nodes = vec![
        Node::Bitmap(Bitmap { transform: Affine::translate((10.0, 20.0)), image: id, smoothing: true }),
        Node::Fill(Fill {
            transform: Affine::IDENTITY,
            path: rect_path(0.0, 0.0, 50.0, 50.0),
            rule: FillRule::NonZero,
            paint: Paint::Bitmap { image: id, transform: Affine::new([2.0, 0.0, 0.0, 2.0, 3.0, 4.0]), repeat: true, smoothing: false },
        }),
    ];
    let out = export_still(&list(nodes), &images).unwrap();
    assert_eq!(out.report.bitmaps, 1);
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(count(&parsed.tags, "DefineBitsLossless"), 1);
    let bitmap = parsed.tags.iter().find_map(|t| if let swf::Tag::DefineBitsLossless(b) = t { Some(b) } else { None }).unwrap();
    assert_eq!((bitmap.version, bitmap.width, bitmap.height), (2, 2, 2));
    assert_eq!(bitmap.format, swf::BitmapFormat::Rgb32);
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&bitmap.data[..]).read_to_end(&mut raw).unwrap();
    assert_eq!(raw, vec![255, 255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 0, 0, 0]);

    let shapes: Vec<_> = parsed.tags.iter().filter_map(|t| if let swf::Tag::DefineShape(s) = t { Some(s) } else { None }).collect();
    let swf::FillStyle::Bitmap { id: bid, matrix, is_smoothed, is_repeating } = shapes[0].styles.fill_styles[0] else { panic!() };
    assert_eq!(bid, bitmap.id);
    assert!(is_smoothed && !is_repeating);
    assert_eq!(matrix.a.to_f64(), 20.0);
    // the bitmap node is a 2x2 pixel rectangle
    assert_eq!(contours(shapes[0]), vec![vec![(0, 0), (40, 0), (40, 40), (0, 40), (0, 0)]]);
    let swf::FillStyle::Bitmap { matrix, is_smoothed, is_repeating, .. } = shapes[1].styles.fill_styles[0] else { panic!() };
    assert!(!is_smoothed && is_repeating);
    assert_eq!((matrix.a.to_f64(), matrix.tx.get(), matrix.ty.get()), (40.0, 60, 80));
}

#[test]
fn missing_images_are_an_error_not_a_panic() {
    let l = list(vec![Node::Bitmap(Bitmap { transform: Affine::IDENTITY, image: anim_display::ImageId(9), smoothing: true })]);
    assert!(matches!(export_still(&l, &ImageStore::new()), Err(anicraft_swf::SwfError::MissingImage(_))));
}

#[test]
fn groups_become_sprites_with_placement_attributes() {
    let child = solid_fill(rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::opaque(0, 255, 0), Affine::translate((5.0, 5.0)));
    let group = Node::Group(Group {
        transform: Affine::new([0.0, 1.0, -1.0, 0.0, 100.0, 100.0]),
        color: ColorTransform { mul: [0.5, 1.0, 1.0, 0.5], add: [0.0, 0.0, 0.2, 0.0] },
        blend: BlendMode::Multiply,
        filters: vec![Filter::Blur { x: 4.0, y: 6.0, quality: 2 }, Filter::ColorMatrix([0.0; 20])],
        children: vec![child.clone(), child],
        ..Group::default()
    });
    let out = export_still(&list(vec![group]), &ImageStore::new()).unwrap();
    // two equal children share one shape, and the sprite holds two placements of it
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(count(&parsed.tags, "DefineShape"), 1);
    let sprite = parsed.tags.iter().find_map(|t| if let swf::Tag::DefineSprite(s) = t { Some(s) } else { None }).unwrap();
    assert_eq!(sprite.num_frames, 1);
    assert_eq!(tag_names(&sprite.tags), vec!["PlaceObject", "PlaceObject", "ShowFrame"]);

    let place = places(&parsed.tags)[0];
    assert_eq!(place.action, swf::PlaceObjectAction::Place(sprite.id));
    let m = place.matrix.unwrap();
    assert_eq!((m.a.to_f64(), m.b.to_f64(), m.c.to_f64(), m.d.to_f64()), (0.0, 1.0, -1.0, 0.0));
    assert_eq!((m.tx.get(), m.ty.get()), (2000, 2000));
    let ct = place.color_transform.unwrap();
    assert_eq!((ct.r_multiply.get(), ct.g_multiply.get(), ct.a_multiply.get()), (128, 256, 128));
    assert_eq!((ct.b_add, ct.r_add), (51, 0));
    assert_eq!(place.blend_mode, Some(swf::BlendMode::Multiply));
    let filters = place.filters.as_ref().unwrap();
    assert_eq!(filters.len(), 2);
    assert!(matches!(&filters[0], swf::Filter::BlurFilter(b) if b.num_passes() == 2 && (b.blur_x.to_f64() - 4.0).abs() < 1e-3));
    assert!(matches!(&filters[1], swf::Filter::ColorMatrixFilter(_)));
}

#[test]
fn masks_use_clip_depth() {
    let mask = vec![solid_fill(rect_path(0.0, 0.0, 20.0, 20.0), Rgba8::BLACK, Affine::IDENTITY)];
    let children = vec![
        solid_fill(rect_path(0.0, 0.0, 50.0, 50.0), Rgba8::opaque(255, 0, 0), Affine::IDENTITY),
        solid_fill(rect_path(10.0, 10.0, 50.0, 50.0), Rgba8::opaque(0, 0, 255), Affine::IDENTITY),
    ];
    let group = Node::Group(Group { mask: Some(mask), children, ..Group::default() });
    let out = export_still(&list(vec![group]), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let sprite = parsed.tags.iter().find_map(|t| if let swf::Tag::DefineSprite(s) = t { Some(s) } else { None }).unwrap();
    let p = places(&sprite.tags);
    assert_eq!(p.len(), 3);
    assert_eq!((p[0].depth, p[0].clip_depth), (1, Some(3)));
    assert_eq!((p[1].depth, p[1].clip_depth), (2, None));
    assert_eq!((p[2].depth, p[2].clip_depth), (3, None));
}

#[test]
fn empty_masks_and_empty_groups_draw_nothing() {
    let child = solid_fill(rect_path(0.0, 0.0, 5.0, 5.0), Rgba8::BLACK, Affine::IDENTITY);
    let nodes = vec![Node::Group(Group { mask: Some(vec![]), children: vec![child], ..Group::default() }), Node::Group(Group::default())];
    let out = export_still(&list(nodes), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(count(&parsed.tags, "PlaceObject"), 0);
}

#[test]
fn animation_reuses_shapes_and_only_moves() {
    let square = rect_path(0.0, 0.0, 10.0, 10.0);
    let frames: Vec<DisplayList> = (0..4)
        .map(|i| {
            let mut nodes = vec![solid_fill(square.clone(), Rgba8::opaque(255, 0, 0), Affine::translate((f64::from(i) * 10.0, 0.0)))];
            if i == 1 || i == 2 {
                nodes.push(solid_fill(square.clone(), Rgba8::opaque(0, 0, 255), Affine::translate((0.0, 50.0))));
            }
            list(nodes)
        })
        .collect();
    let out = export_display_lists(&frames, &ImageStore::new(), ExportOptions::for_list(&frames[0], 12.0)).unwrap();
    assert_eq!(out.report.frames, 4);
    let buf = parse(&out.swf);
    assert_eq!(buf.header.num_frames(), 4);
    assert_eq!(buf.header.frame_rate().to_f32(), 12.0);
    let parsed = swf::parse_swf(&buf).unwrap();
    // red and blue squares: two shapes in total, whatever the frame count
    assert_eq!(count(&parsed.tags, "DefineShape"), 2);
    // frame 0 places red; frame 1 moves red and places blue; frame 2 moves red; frame 3 moves red and removes blue
    let names = tag_names(&parsed.tags);
    assert_eq!(count(&parsed.tags, "ShowFrame"), 4);
    assert_eq!(count(&parsed.tags, "RemoveObject"), 1);
    // 1 initial place + 2 for blue? no: red placed once, blue placed once; red moved on frames 1..=3
    let all = places(&parsed.tags);
    assert_eq!(all.len(), 2 + 3);
    let moves: Vec<_> = all.iter().filter(|p| p.action == swf::PlaceObjectAction::Modify).collect();
    assert_eq!(moves.len(), 3);
    assert_eq!(moves[2].matrix.unwrap().tx.get(), 600);
    assert!(moves.iter().all(|m| m.depth == 1 && m.color_transform.is_none()));
    assert!(names.contains(&"RemoveObject"));
}

#[test]
fn replacing_a_character_at_a_depth_removes_then_places() {
    let a = list(vec![solid_fill(rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::opaque(255, 0, 0), Affine::IDENTITY)]);
    let b = list(vec![solid_fill(rect_path(0.0, 0.0, 20.0, 20.0), Rgba8::opaque(255, 0, 0), Affine::IDENTITY)]);
    let out = export_display_lists([&a, &b], &ImageStore::new(), ExportOptions::for_list(&a, 24.0)).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let names = tag_names(&parsed.tags);
    let tail: Vec<_> = names.iter().skip_while(|n| **n != "ShowFrame").collect();
    assert_eq!(tail, vec![&"ShowFrame", &"DefineShape", &"RemoveObject", &"PlaceObject", &"ShowFrame"]);
}

#[test]
fn non_looping_exports_stop_on_the_last_frame() {
    let a = list(vec![solid_fill(rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::BLACK, Affine::IDENTITY)]);
    let mut options = ExportOptions::for_list(&a, 24.0);
    options.loop_playback = false;
    options.movie.compression = Compression::None;
    let out = export_display_lists([&a, &a], &ImageStore::new(), options).unwrap();
    assert_eq!(&out.swf[..3], b"FWS");
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    let names = tag_names(&parsed.tags);
    assert_eq!(count(&parsed.tags, "DoAction"), 1);
    let stop_at = names.iter().position(|n| *n == "DoAction").unwrap();
    // the stop sits in the last frame, i.e. after the first ShowFrame
    assert_eq!(names.iter().take(stop_at).filter(|n| **n == "ShowFrame").count(), 1);
    let swf::Tag::DoAction(action) = &parsed.tags[stop_at] else { panic!() };
    assert_eq!(**action, ACTION_STOP[..]);
}

#[test]
fn meshes_are_emulated_with_masked_triangles() {
    let content = vec![solid_fill(rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::opaque(255, 0, 0), Affine::IDENTITY)];
    let mesh = Mesh {
        transform: Affine::translate((5.0, 5.0)),
        content,
        source: vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.0, 10.0), Point::new(10.0, 10.0)],
        target: vec![Point::new(0.0, 0.0), Point::new(20.0, 0.0), Point::new(0.0, 10.0), Point::new(20.0, 10.0)],
        triangles: vec![[0, 1, 2], [1, 3, 2], [0, 0, 0], [0, 1, 99]],
    };
    let out = export_still(&list(vec![Node::Mesh(mesh)]), &ImageStore::new()).unwrap();
    assert!(out.report.notes.contains(&ExportNote::MeshEmulated));
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    // the content shape, two triangle masks (degenerate and out-of-range triangles are skipped)
    assert_eq!(count(&parsed.tags, "DefineShape"), 3);
    // content sprite, two triangle sprites, one mesh sprite
    assert_eq!(count(&parsed.tags, "DefineSprite"), 4);
}

#[test]
fn deep_and_wide_lists_do_not_blow_up() {
    let many: Vec<Node> = (0..300).map(|i| solid_fill(rect_path(f64::from(i), 0.0, 1.0, 1.0), Rgba8::BLACK, Affine::IDENTITY)).collect();
    let out = export_still(&list(many), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(places(&parsed.tags).len(), 300);
    assert_eq!(places(&parsed.tags)[299].depth, 300);
}

#[test]
fn movie_api_sounds_labels_scenes_and_streams() {
    let mp3: Vec<u8> = (0..30)
        .flat_map(|_| {
            let mut f = vec![0xff, 0xfb, 0x90, 0x00];
            f.resize(417, 0);
            f
        })
        .collect();
    let mut movie =
        Movie::new(MovieSettings { compression: Compression::None, metadata: Some("<x:xmpmeta/>".into()), use_network: true, ..MovieSettings::default() })
            .unwrap();
    let sound = movie.define_sound_mp3(&mp3).unwrap();
    let pcm = movie.define_sound_pcm(&[0, 1000, -1000, 0], 2, 22_050).unwrap();
    let stream: Mp3Stream = split_mp3_stream(&mp3, 24.0, 100).unwrap();
    movie.begin_scene("Intro");
    let mut first = Frame { label: Some("start".into()), items: vec![FrameItem::StreamHead(stream.head_body())] };
    first.items.push(FrameItem::StartSound { sound, info: SoundInfo { loops: Some(2), ..SoundInfo::default() } });
    first.items.push(FrameItem::StartSound { sound: pcm, info: SoundInfo::default() });
    for (i, block) in stream.blocks.iter().enumerate() {
        let mut frame = if i == 0 { first.clone() } else { Frame::default() };
        frame.items.push(FrameItem::StreamBlock(block.clone()));
        movie.add_frame(&frame).unwrap();
    }
    let n = stream.blocks.len();
    let file = movie.finish().unwrap();
    let buf = parse(&file);
    assert!(buf.header.has_metadata() && buf.header.use_network_sandbox());
    assert_eq!(usize::from(buf.header.num_frames()), n);
    let parsed = swf::parse_swf(&buf).unwrap();
    let sounds: Vec<_> = parsed.tags.iter().filter_map(|t| if let swf::Tag::DefineSound(s) = t { Some(s) } else { None }).collect();
    assert_eq!(sounds.len(), 2);
    assert_eq!(sounds[0].format.compression, swf::AudioCompression::Mp3);
    assert_eq!(sounds[0].format.sample_rate, 44_100);
    assert!(sounds[0].format.is_stereo && sounds[0].format.is_16_bit);
    assert_eq!(sounds[0].num_samples, 30 * 1152);
    assert_eq!(sounds[1].format.compression, swf::AudioCompression::Uncompressed);
    assert_eq!(sounds[1].format.sample_rate, 22_050);
    assert_eq!(sounds[1].num_samples, 2);
    let head = parsed.tags.iter().find_map(|t| if let swf::Tag::SoundStreamHead2(h) = t { Some(h) } else { None }).unwrap();
    assert_eq!(head.stream_format.compression, swf::AudioCompression::Mp3);
    assert_eq!(head.num_samples_per_block, 1838);
    assert_eq!(count(&parsed.tags, "SoundStreamBlock"), n);
    let starts: Vec<_> = parsed.tags.iter().filter_map(|t| if let swf::Tag::StartSound(s) = t { Some(s) } else { None }).collect();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[0].sound_info.num_loops, 2);
    let scene = parsed.tags.iter().find_map(|t| if let swf::Tag::DefineSceneAndFrameLabelData(d) = t { Some(d) } else { None }).unwrap();
    assert_eq!(scene.scenes.len(), 1);
    assert_eq!(scene.scenes[0].label.to_str_lossy(swf::UTF_8), "Intro");
    assert_eq!(scene.frame_labels[0].label.to_str_lossy(swf::UTF_8), "start");
}

#[test]
fn sprites_with_their_own_timelines_and_names() {
    let mut movie = Movie::new(MovieSettings { compression: Compression::None, ..MovieSettings::default() }).unwrap();
    let mut b = ShapeBuilder::new();
    let fill = b.add_fill(FillStyle::Solid(Rgba8::opaque(9, 9, 9))).unwrap();
    b.add_path(&rect_path(0.0, 0.0, 4.0, 4.0), PathStyle { fill1: fill, ..PathStyle::default() });
    let shape = movie.define_shape(&b.finish()).unwrap();
    let clip = movie
        .define_sprite(&[
            Frame { label: Some("a".into()), items: vec![FrameItem::Place(Placement::new(1, shape, Matrix::IDENTITY))] },
            Frame::new(vec![FrameItem::Place(Placement {
                depth: 1,
                is_move: true,
                matrix: Some(Matrix::from_affine(Affine::translate((10.0, 0.0)))),
                ..Placement::default()
            })]),
            Frame::new(vec![FrameItem::DoAction(ACTION_STOP.to_vec())]),
        ])
        .unwrap();
    movie.export_asset(clip, "walker");
    let mut place = Placement::new(1, clip, Matrix::IDENTITY);
    place.name = Some("hero".into());
    place.visible = Some(true);
    place.cache_as_bitmap = Some(true);
    movie.add_frame(&Frame::new(vec![FrameItem::Place(place)])).unwrap();
    let buf = parse(&movie.finish().unwrap());
    let parsed = swf::parse_swf(&buf).unwrap();
    let sprite = parsed.tags.iter().find_map(|t| if let swf::Tag::DefineSprite(s) = t { Some(s) } else { None }).unwrap();
    assert_eq!(sprite.num_frames, 3);
    assert_eq!(tag_names(&sprite.tags), vec!["FrameLabel", "PlaceObject", "ShowFrame", "PlaceObject", "ShowFrame", "DoAction", "ShowFrame"]);
    let place = places(&parsed.tags)[0];
    assert_eq!(place.name.map(|n| n.to_str_lossy(swf::UTF_8).into_owned()), Some("hero".to_string()));
    assert_eq!(place.is_bitmap_cached, Some(true));
    assert_eq!(place.is_visible, Some(true));
    let exports = parsed.tags.iter().find_map(|t| if let swf::Tag::ExportAssets(e) = t { Some(e) } else { None }).unwrap();
    assert_eq!(exports[0].id, clip.0);
    assert_eq!(CharacterId(exports[0].id), clip);
}

#[test]
fn a_fill_followed_by_a_stroke_of_the_same_path_is_one_shape() {
    let path = rect_path(5.0, 5.0, 30.0, 30.0);
    let stroke = Node::Stroke(Stroke {
        transform: Affine::translate((1.0, 2.0)),
        path: path.clone(),
        style: StrokeStyle { width: 2.0, ..StrokeStyle::default() },
        paint: Paint::Solid(Rgba8::BLACK),
    });
    let fill = solid_fill(path.clone(), Rgba8::opaque(255, 0, 0), Affine::translate((1.0, 2.0)));
    let out = export_still(&list(vec![fill.clone(), stroke.clone()]), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(count(&parsed.tags, "DefineShape"), 1);
    assert_eq!(count(&parsed.tags, "PlaceObject"), 1);
    let shape = first_shape(&parsed.tags);
    assert_eq!((shape.styles.fill_styles.len(), shape.styles.line_styles.len()), (1, 1));
    // the bounds include half the stroke width, the edge bounds do not
    assert_eq!((shape.edge_bounds.x_min.get(), shape.shape_bounds.x_min.get() < shape.edge_bounds.x_min.get()), (100, true));

    // a stroke before the fill, or with another transform, is a separate shape
    let out = export_still(&list(vec![stroke, fill.clone()]), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    assert_eq!(count(&swf::parse_swf(&buf).unwrap().tags, "DefineShape"), 2);
    let moved = Node::Stroke(Stroke { transform: Affine::IDENTITY, path, style: StrokeStyle::default(), paint: Paint::Solid(Rgba8::BLACK) });
    let out = export_still(&list(vec![fill, moved]), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    assert_eq!(count(&swf::parse_swf(&buf).unwrap().tags, "DefineShape"), 2);
}

#[test]
fn images_with_identical_pixels_share_one_bitmap_character() {
    let mut images = ImageStore::new();
    let a = images.insert(Image::new(1, 1, vec![9, 9, 9, 255]).unwrap());
    let b = images.insert(Image::new(1, 1, vec![9, 9, 9, 255]).unwrap());
    let c = images.insert(Image::new(1, 1, vec![9, 9, 10, 255]).unwrap());
    let nodes = [a, b, c].into_iter().map(|image| Node::Bitmap(Bitmap { transform: Affine::IDENTITY, image, smoothing: true })).collect();
    let out = export_still(&list(nodes), &images).unwrap();
    assert_eq!(out.report.bitmaps, 2);
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(count(&parsed.tags, "DefineBitsLossless"), 2);
    // the first two bitmap nodes are the same shape character too
    assert_eq!(count(&parsed.tags, "DefineShape"), 2);
    assert_eq!(count(&parsed.tags, "PlaceObject"), 3);
}

#[test]
fn deserialised_paths_that_do_not_start_with_a_move_are_repaired() {
    // BezPath's serde implementation does not enforce kurbo's "starts with MoveTo" rule
    let path: BezPath =
        serde_json::from_value(serde_json::json!([{"LineTo": {"x": 10.0, "y": 0.0}}, {"LineTo": {"x": 10.0, "y": 10.0}}, "ClosePath"])).unwrap();
    assert!(!matches!(path.elements().first(), Some(kurbo::PathEl::MoveTo(_))));
    let dashed = StrokeStyle { width: 1.0, dashes: vec![2.0, 2.0], ..StrokeStyle::default() };
    let nodes = vec![
        Node::Fill(Fill { transform: Affine::IDENTITY, path: path.clone(), rule: FillRule::NonZero, paint: Paint::Solid(Rgba8::BLACK) }),
        Node::Stroke(Stroke { transform: Affine::IDENTITY, path, style: dashed, paint: Paint::Solid(Rgba8::BLACK) }),
    ];
    let out = export_still(&list(nodes), &ImageStore::new()).unwrap();
    let buf = parse(&out.swf);
    assert!(count(&swf::parse_swf(&buf).unwrap().tags, "DefineShape") >= 1);
}

#[test]
fn degenerate_dashed_paths_do_not_panic() {
    // kurbo's dash iterator can yield elements that do not start with a MoveTo for degenerate input
    let point = |x: f64, y: f64| Point::new(x, y);
    let paths = [
        vec![kurbo::PathEl::MoveTo(point(1.0, 1.0))],
        vec![kurbo::PathEl::MoveTo(point(1.0, 1.0)), kurbo::PathEl::ClosePath],
        vec![kurbo::PathEl::MoveTo(point(1.0, 1.0)), kurbo::PathEl::LineTo(point(1.0, 1.0))],
        vec![kurbo::PathEl::MoveTo(point(0.0, 0.0)), kurbo::PathEl::LineTo(point(5.0, 0.0)), kurbo::PathEl::ClosePath],
        vec![kurbo::PathEl::MoveTo(point(0.0, 0.0)), kurbo::PathEl::LineTo(point(5.0, 0.0)), kurbo::PathEl::MoveTo(point(7.0, 7.0))],
    ];
    for elements in paths {
        for dashes in [vec![0.0, 3.0], vec![1.0], vec![3.0, 0.0, 2.0], vec![0.5, 0.5]] {
            let stroke = Node::Stroke(Stroke {
                transform: Affine::IDENTITY,
                path: BezPath::from_vec(elements.clone()),
                style: StrokeStyle { width: 1.0, dashes, ..StrokeStyle::default() },
                paint: Paint::Solid(Rgba8::BLACK),
            });
            let out = export_still(&list(vec![stroke]), &ImageStore::new()).unwrap();
            let buf = parse(&out.swf);
            swf::parse_swf(&buf).unwrap();
        }
    }
}

#[test]
fn options_and_reports_round_trip_through_json() {
    let options = ExportOptions::for_list(&list(vec![]), 30.0);
    let json = serde_json::to_string(&options).unwrap();
    assert_eq!(serde_json::from_str::<ExportOptions>(&json).unwrap(), options);
    let out = export_still(&list(vec![solid_fill(rect_path(0.0, 0.0, 1.0, 1.0), Rgba8::BLACK, Affine::IDENTITY)]), &ImageStore::new()).unwrap();
    let json = serde_json::to_string(&out.report).unwrap();
    assert_eq!(serde_json::from_str::<anicraft_swf::ExportReport>(&json).unwrap(), out.report);
}

#[test]
fn buttons_read_back_with_their_states_and_scripts() {
    use anicraft_swf::button::{ButtonAction, ButtonDef, ButtonRecord, ButtonStates, condition};

    let mut movie = Movie::new(MovieSettings { compression: Compression::None, ..MovieSettings::default() }).unwrap();
    let shape = |size: f64, color: Rgba8| {
        let mut b = ShapeBuilder::new();
        let fill = b.add_fill(FillStyle::Solid(color)).unwrap();
        b.add_path(&rect_path(0.0, 0.0, size, size), PathStyle { fill1: fill, ..PathStyle::default() });
        b.finish()
    };
    let up = movie.define_shape(&shape(40.0, Rgba8::opaque(255, 0, 0))).unwrap();
    let over = movie.define_shape(&shape(40.0, Rgba8::opaque(0, 0, 255))).unwrap();
    let hit = movie.define_shape(&shape(60.0, Rgba8::BLACK)).unwrap();
    let button = movie
        .define_button(&ButtonDef {
            track_as_menu: true,
            records: vec![
                ButtonRecord::new(ButtonStates::UP, up, 1),
                ButtonRecord {
                    matrix: Matrix::from_affine(Affine::translate((2.0, 3.0))),
                    ..ButtonRecord::new(ButtonStates { over: true, down: true, ..ButtonStates::default() }, over, 1)
                },
                ButtonRecord::new(ButtonStates::HIT, hit, 1),
            ],
            actions: vec![ButtonAction { conditions: condition::OVER_DOWN_TO_OVER_UP, bytecode: ACTION_STOP.to_vec() }],
        })
        .unwrap();
    movie.add_frame(&Frame::new(vec![FrameItem::Place(Placement::new(1, button, Matrix::IDENTITY))])).unwrap();
    let buf = parse(&movie.finish().unwrap());
    let parsed = swf::parse_swf(&buf).unwrap();
    let swf::Tag::DefineButton2(b) = parsed.tags.iter().find(|t| matches!(t, swf::Tag::DefineButton2(_))).unwrap() else { panic!() };
    assert!(b.is_track_as_menu);
    assert_eq!(b.records.len(), 3);
    assert_eq!(b.records[0].states, swf::ButtonState::UP);
    assert_eq!(b.records[1].states, swf::ButtonState::OVER | swf::ButtonState::DOWN);
    assert_eq!(b.records[1].matrix.tx.get(), 40);
    assert_eq!(b.records[2].states, swf::ButtonState::HIT_TEST);
    assert_eq!((b.records[2].id, b.records[2].depth), (hit.0, 1));
    assert_eq!(b.actions.len(), 1);
    assert_eq!(b.actions[0].conditions, swf::ButtonActionCondition::OVER_DOWN_TO_OVER_UP);
    assert_eq!(b.actions[0].action_data, &ACTION_STOP[..]);
}

#[test]
fn exporter_streams_frames_one_at_a_time() {
    let l = list(vec![solid_fill(rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::BLACK, Affine::IDENTITY)]);
    let mut exporter = DisplayListExporter::new(ExportOptions::for_list(&l, 30.0)).unwrap();
    for _ in 0..5 {
        exporter.push_frame(&l, &ImageStore::new()).unwrap();
    }
    let out = exporter.finish().unwrap();
    assert_eq!(out.report.frames, 5);
    assert_eq!(out.report.characters, 1);
    let buf = parse(&out.swf);
    let parsed = swf::parse_swf(&buf).unwrap();
    assert_eq!(count(&parsed.tags, "ShowFrame"), 5);
    assert_eq!(places(&parsed.tags).len(), 1);
}
