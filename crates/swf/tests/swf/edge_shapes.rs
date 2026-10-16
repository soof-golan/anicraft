//! Animate edge shapes become native SWF shapes: structure checks with Ruffle's parser, and (when
//! `ANICRAFT_RUFFLE_EXPORTER` is set) pixel checks with Ruffle's renderer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn
)]

use crate::common;

use anicraft_swf::edge::{NoBitmaps, shape_from_edges};
use anicraft_swf::movie::{Compression, Frame, FrameItem, Movie, MovieSettings};
use anicraft_swf::place::Placement;
use anicraft_swf::types::Matrix;
use anim_core::{Affine, Point, Rgba8};
use anim_display::{GradientStop, Image, Interpolation, Spread};
use anim_shape::{BitmapFill, Edge, EdgeShape, FillStyle, GradientFill, Segment, StrokeStyle, parse_edges};
use common::{Rendered, render_swf};

const WHITE: [u8; 3] = [255, 255, 255];

/// A line segment between points given in pixels (edge coordinates are twips).
fn seg(x0: f64, y0: f64, x1: f64, y1: f64) -> Segment {
    Segment::Line { from: Point::new(x0 * 20.0, y0 * 20.0), to: Point::new(x1 * 20.0, y1 * 20.0) }
}

fn red() -> FillStyle {
    FillStyle::Solid(Rgba8::opaque(255, 0, 0))
}

fn blue() -> FillStyle {
    FillStyle::Solid(Rgba8::opaque(0, 0, 255))
}

/// A 100x100 white movie showing `conversion` at depth 1 (and defining bitmaps first when asked to).
fn movie_of(shape: &EdgeShape, bitmaps: &[Image]) -> Vec<u8> {
    let mut movie = Movie::new(MovieSettings { width: 100.0, height: 100.0, compression: Compression::None, ..MovieSettings::default() }).unwrap();
    let ids: Vec<_> = bitmaps.iter().map(|image| movie.define_bitmap(image).unwrap()).collect();
    let resolver = |path: &str| path.strip_prefix("bitmap").and_then(|n| n.parse::<usize>().ok()).and_then(|n| ids.get(n).copied());
    let out = shape_from_edges(shape, &resolver).unwrap();
    let id = movie.define_shape(&out.shape).unwrap();
    movie.add_frame(&Frame::new(vec![FrameItem::Place(Placement::new(1, id, Matrix::IDENTITY))])).unwrap();
    movie.finish().unwrap()
}

/// Two squares side by side that share their middle edge, which carries a different fill on each side.
fn two_squares(stroke_the_shared_edge: bool) -> EdgeShape {
    EdgeShape {
        fills: vec![red(), blue()],
        strokes: vec![StrokeStyle::plain(6.0, Rgba8::BLACK)],
        edges: vec![
            // the red square's outline without the shared side, clockwise: its inside is on the right
            Edge::new(0, 1, 0, vec![seg(60.0, 60.0, 20.0, 60.0), seg(20.0, 60.0, 20.0, 20.0), seg(20.0, 20.0, 60.0, 20.0)]),
            // the shared side, travelling down: red on the right, blue on the left
            Edge::new(2, 1, u32::from(stroke_the_shared_edge), vec![seg(60.0, 20.0, 60.0, 60.0)]),
            // the blue square's outline without the shared side, clockwise
            Edge::new(0, 2, 0, vec![seg(60.0, 20.0, 100.0, 20.0), seg(100.0, 20.0, 100.0, 60.0), seg(100.0, 60.0, 60.0, 60.0)]),
        ],
    }
}

#[test]
fn edge_records_reach_the_file_with_their_style_indices() {
    let swf = movie_of(&two_squares(true), &[]);
    let buf = swf::decompress_swf(&swf[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    let shape = parsed.tags.iter().find_map(|t| if let swf::Tag::DefineShape(s) = t { Some(s) } else { None }).unwrap();
    assert_eq!((shape.styles.fill_styles.len(), shape.styles.line_styles.len()), (2, 1));
    assert_eq!(shape.styles.line_styles[0].width().get(), 120);
    let changes: Vec<_> = shape
        .shape
        .iter()
        .filter_map(|r| {
            if let swf::ShapeRecord::StyleChange(c) = r {
                Some((c.move_to.map(|p| (p.x.get(), p.y.get())), c.fill_style_0, c.fill_style_1, c.line_style))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        changes,
        vec![(Some((1200, 1200)), None, Some(1), None), (Some((1200, 400)), Some(2), None, Some(1)), (Some((1200, 400)), Some(0), Some(2), Some(0)),]
    );
    // nothing was closed: 3 + 1 + 3 straight edges
    assert_eq!(shape.shape.iter().filter(|r| matches!(r, swf::ShapeRecord::StraightEdge { .. })).count(), 7);
}

fn assert_two_squares(r: &Rendered) {
    r.assert_near(40, 40, [255, 0, 0], 3, "red square");
    r.assert_near(80, 40, [0, 0, 255], 3, "blue square");
    r.assert_near(40, 70, WHITE, 3, "below");
    r.assert_near(10, 40, WHITE, 3, "left of");
    r.assert_near(99, 10, WHITE, 3, "right of");
}

#[test]
fn a_shared_edge_fills_both_regions() {
    let Some(r) = render_swf("edge_two_squares", &movie_of(&two_squares(false), &[])) else { return };
    assert_two_squares(&r);
    // no gap between the regions
    r.assert_near(59, 40, [255, 0, 0], 40, "just left of the shared edge");
    r.assert_near(61, 40, [0, 0, 255], 40, "just right of the shared edge");
}

#[test]
fn a_stroked_shared_edge_is_drawn_over_both_fills() {
    let Some(r) = render_swf("edge_stroked", &movie_of(&two_squares(true), &[])) else { return };
    r.assert_near(40, 40, [255, 0, 0], 3, "red square");
    r.assert_near(80, 40, [0, 0, 255], 3, "blue square");
    r.assert_near(60, 40, [0, 0, 0], 20, "the stroke");
    r.assert_near(58, 40, [0, 0, 0], 40, "inside the stroke");
}

#[test]
fn cubic_circles_and_xfl_edge_text_render() {
    // a circle of radius 30 around (50, 50) from four cubics, clockwise
    let k = 0.552_284_75 * 30.0;
    let c = |x: f64, y: f64| Point::new(x * 20.0, y * 20.0);
    let circle = vec![
        Segment::Cubic { from: c(80.0, 50.0), ctrl1: c(80.0, 50.0 + k), ctrl2: c(50.0 + k, 80.0), to: c(50.0, 80.0) },
        Segment::Cubic { from: c(50.0, 80.0), ctrl1: c(50.0 - k, 80.0), ctrl2: c(20.0, 50.0 + k), to: c(20.0, 50.0) },
        Segment::Cubic { from: c(20.0, 50.0), ctrl1: c(20.0, 50.0 - k), ctrl2: c(50.0 - k, 20.0), to: c(50.0, 20.0) },
        Segment::Cubic { from: c(50.0, 20.0), ctrl1: c(50.0 + k, 20.0), ctrl2: c(80.0, 50.0 - k), to: c(80.0, 50.0) },
    ];
    let shape = EdgeShape { fills: vec![red()], strokes: vec![], edges: vec![Edge::new(0, 1, 0, circle)] };
    let Some(r) = render_swf("edge_circle", &movie_of(&shape, &[])) else { return };
    r.assert_near(50, 50, [255, 0, 0], 3, "centre");
    r.assert_near(50, 25, [255, 0, 0], 3, "near the top");
    r.assert_near(77, 50, [255, 0, 0], 3, "near the right");
    r.assert_near(25, 25, WHITE, 3, "outside the circle, inside its bounding box");

    // the same square as XFL spells it
    let text = parse_edges("!400 400|1600 400|1600 1600|400 1600|400 400").unwrap();
    let square = EdgeShape { fills: vec![blue()], strokes: vec![], edges: vec![Edge::new(0, 1, 0, text)] };
    let Some(r) = render_swf("edge_xfl_square", &movie_of(&square, &[])) else { return };
    r.assert_near(50, 50, [0, 0, 255], 3, "inside");
    r.assert_near(10, 10, WHITE, 3, "outside");
}

#[test]
fn bitmap_and_gradient_fills_from_edge_styles() {
    let image = Image::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    // each bitmap pixel covers 25 x 50 px: a stored matrix has SWF's scale (20 per pixel) times the scale, and its
    // translation in pixels
    let bitmap = FillStyle::Bitmap(BitmapFill {
        path: "bitmap0".into(),
        matrix: Affine::new([20.0 * 25.0, 0.0, 0.0, 20.0 * 50.0, 10.0, 10.0]),
        clipped: true,
        smoothing: false,
    });
    let gradient = FillStyle::RadialGradient(GradientFill {
        matrix: Affine::translate((75.0, 75.0)) * Affine::scale(20.0 / anim_display::GRADIENT_EXTENT),
        stops: vec![GradientStop { offset: 0.0, color: Rgba8::opaque(255, 255, 0) }, GradientStop { offset: 1.0, color: Rgba8::opaque(0, 0, 0) }],
        spread: Spread::Pad,
        interpolation: Interpolation::Rgb,
        focal: 0.0,
    });
    let shape = EdgeShape {
        fills: vec![bitmap, gradient],
        strokes: vec![],
        edges: vec![
            Edge::new(0, 1, 0, vec![seg(10.0, 10.0, 60.0, 10.0), seg(60.0, 10.0, 60.0, 60.0), seg(60.0, 60.0, 10.0, 60.0), seg(10.0, 60.0, 10.0, 10.0)]),
            Edge::new(0, 2, 0, vec![seg(55.0, 55.0, 95.0, 55.0), seg(95.0, 55.0, 95.0, 95.0), seg(95.0, 95.0, 55.0, 95.0), seg(55.0, 95.0, 55.0, 55.0)]),
        ],
    };
    let Some(r) = render_swf("edge_bitmap_gradient", &movie_of(&shape, &[image])) else { return };
    r.assert_near(20, 30, [255, 0, 0], 3, "first bitmap pixel");
    r.assert_near(50, 30, [0, 255, 0], 3, "second bitmap pixel");
    r.assert_near(75, 75, [255, 255, 0], 10, "gradient centre");
    r.assert_near(93, 93, [0, 0, 0], 40, "gradient corner (beyond its radius)");
    r.assert_near(5, 90, WHITE, 3, "outside");

    // an unresolved bitmap turns grey and says so
    let missing = shape_from_edges(&shape, &NoBitmaps).unwrap();
    assert_eq!(missing.notes, vec![anicraft_swf::ExportNote::MissingBitmap]);
}
