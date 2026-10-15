//! Shape tweens: structure checks with Ruffle's parser and, when `ANICRAFT_RUFFLE_EXPORTER` is set, pixel checks of
//! the blend at several ratios.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn
)]

mod common;

use anicraft_swf::morph::MorphShapeDef;
use anicraft_swf::movie::{Compression, Frame, FrameItem, Movie, MovieSettings};
use anicraft_swf::place::Placement;
use anicraft_swf::shape::{PathStyle, ShapeBuilder, ShapeDef, filled_path_shape};
use anicraft_swf::style::{FillStyle, Gradient, LineStyle};
use anicraft_swf::types::Matrix;
use anim_core::kurbo::{BezPath, Rect, Shape};
use anim_core::{Affine, Rgba8};
use anim_display::{FillRule, GradientStop, Interpolation, Spread};
use common::render_swf_frames;

const WHITE: [u8; 3] = [255, 255, 255];

fn square(x: f64, y: f64, size: f64, fill: FillStyle) -> ShapeDef {
    filled_path_shape(&Rect::new(x, y, x + size, y + size).to_path(0.1), fill, FillRule::NonZero).unwrap()
}

/// A 100x100 movie that shows `morph` at each of `ratios`, one frame per ratio.
fn ratio_frames(morph: &MorphShapeDef, ratios: &[u16]) -> Vec<u8> {
    let mut movie = Movie::new(MovieSettings { width: 100.0, height: 100.0, compression: Compression::None, ..MovieSettings::default() }).unwrap();
    let id = movie.define_morph_shape(morph).unwrap();
    for (index, ratio) in ratios.iter().enumerate() {
        let placement = if index == 0 {
            Placement { ratio: Some(*ratio), ..Placement::new(1, id, Matrix::IDENTITY) }
        } else {
            Placement { depth: 1, is_move: true, ratio: Some(*ratio), ..Placement::default() }
        };
        movie.add_frame(&Frame::new(vec![FrameItem::Place(placement)])).unwrap();
    }
    movie.finish().unwrap()
}

fn ramp(left: Rgba8, right: Rgba8, from_x: f64, width: f64) -> FillStyle {
    let stops = [GradientStop { offset: 0.0, color: left }, GradientStop { offset: 1.0, color: right }];
    FillStyle::Linear {
        matrix: Matrix::from_affine(Affine::translate((from_x + width / 2.0, 50.0)) * Affine::scale(width / 2.0 / anim_display::GRADIENT_EXTENT)),
        gradient: Gradient::from_stops(&stops, Spread::Pad, Interpolation::Rgb).0,
    }
}

#[test]
fn morph_shapes_parse_back_with_both_ends() {
    let a = square(10.0, 10.0, 20.0, FillStyle::Solid(Rgba8::opaque(255, 0, 0)));
    let b = square(30.0, 30.0, 60.0, FillStyle::Solid(Rgba8::opaque(0, 0, 255)));
    let swf = ratio_frames(&MorphShapeDef::new(&a, &b).unwrap(), &[0, 65535]);
    let buf = swf::decompress_swf(&swf[..]).unwrap();
    let parsed = swf::parse_swf(&buf).unwrap();
    let morph = parsed.tags.iter().find_map(|t| if let swf::Tag::DefineMorphShape(m) = t { Some(m) } else { None }).expect("a morph shape");
    assert_eq!(morph.version, 2);
    assert_eq!((morph.start.shape_bounds.x_min.get(), morph.start.shape_bounds.x_max.get()), (200, 600));
    assert_eq!((morph.end.shape_bounds.x_min.get(), morph.end.shape_bounds.x_max.get()), (600, 1800));
    assert_eq!(morph.start.fill_styles.len(), 1);
    assert_eq!(morph.end.fill_styles.len(), 1);
    // a move and four edges on both sides
    assert_eq!(morph.start.shape.len(), 5);
    assert_eq!(morph.end.shape.len(), 5);
}

#[test]
fn colours_and_geometry_blend_with_the_ratio() {
    let a = square(10.0, 10.0, 20.0, FillStyle::Solid(Rgba8::opaque(255, 0, 0)));
    let b = square(30.0, 30.0, 60.0, FillStyle::Solid(Rgba8::opaque(0, 0, 255)));
    let swf = ratio_frames(&MorphShapeDef::new(&a, &b).unwrap(), &[0, 32768, 65535]);
    let Some(frames) = render_swf_frames("morph_solid", &swf, 3) else { return };
    frames[0].assert_near(20, 20, [255, 0, 0], 3, "start: red square");
    frames[0].assert_near(60, 60, WHITE, 3, "start: nothing where the end square is");
    // halfway: the square spans 20..60 and is purple
    frames[1].assert_near(40, 40, [127, 0, 127], 6, "middle: purple");
    frames[1].assert_near(15, 15, WHITE, 3, "middle: outside");
    frames[1].assert_near(70, 70, WHITE, 3, "middle: outside");
    frames[2].assert_near(60, 60, [0, 0, 255], 3, "end: blue square");
    frames[2].assert_near(20, 20, WHITE, 3, "end: nothing where the start square was");
}

#[test]
fn gradients_morph() {
    let a = square(10.0, 10.0, 40.0, ramp(Rgba8::opaque(255, 0, 0), Rgba8::opaque(0, 255, 0), 10.0, 40.0));
    let b = square(10.0, 10.0, 80.0, ramp(Rgba8::opaque(0, 0, 255), Rgba8::opaque(255, 255, 0), 10.0, 80.0));
    let swf = ratio_frames(&MorphShapeDef::new(&a, &b).unwrap(), &[0, 65535]);
    let Some(frames) = render_swf_frames("morph_gradient", &swf, 2) else { return };
    frames[0].assert_near(12, 30, [240, 15, 0], 30, "start: red end of the ramp");
    frames[0].assert_near(48, 30, [15, 240, 0], 30, "start: green end of the ramp");
    frames[1].assert_near(14, 60, [15, 0, 240], 40, "end: blue end of the ramp");
    frames[1].assert_near(86, 60, [240, 240, 15], 40, "end: yellow end of the ramp");
}

#[test]
fn strokes_morph_in_width_and_colour() {
    let line = |to_x: f64, width: u16, color: Rgba8| {
        let mut path = BezPath::new();
        path.move_to((10.0, 50.0));
        path.line_to((to_x, 50.0));
        let mut builder = ShapeBuilder::new();
        let index = builder.add_line(LineStyle::solid(width, color)).unwrap();
        builder.add_path(&path, PathStyle { line: index, ..PathStyle::default() });
        builder.finish()
    };
    let a = line(50.0, 40, Rgba8::BLACK);
    let b = line(90.0, 200, Rgba8::opaque(255, 0, 255));
    let swf = ratio_frames(&MorphShapeDef::new(&a, &b).unwrap(), &[0, 65535]);
    let Some(frames) = render_swf_frames("morph_stroke", &swf, 2) else { return };
    frames[0].assert_near(30, 50, [0, 0, 0], 20, "start: thin black line");
    frames[0].assert_near(30, 56, WHITE, 3, "start: below the thin line");
    frames[0].assert_near(70, 50, WHITE, 3, "start: past the line's end");
    frames[1].assert_near(70, 50, [255, 0, 255], 20, "end: magenta line");
    frames[1].assert_near(70, 54, [255, 0, 255], 20, "end: the thick line covers it");
    frames[1].assert_near(70, 66, WHITE, 3, "end: below the thick line");
}
