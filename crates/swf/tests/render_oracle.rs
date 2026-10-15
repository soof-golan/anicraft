//! Renders exported SWFs with Ruffle's `exporter` (an independent SWF renderer) and checks pixels.
//!
//! Set `ANICRAFT_RUFFLE_EXPORTER` to the path of a built `exporter` binary (`cargo build -p exporter` in a Ruffle
//! checkout) to run these; without it every test returns early, so CI stays green.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::missing_const_for_fn
)]

mod common;

use anicraft_swf::export::{ExportOptions, export_display_lists};
use anicraft_swf::movie::Compression;
use anim_core::kurbo::{BezPath, Circle, Rect, Shape};
use anim_core::{Affine, ColorTransform, Rgba8, Size};
use anim_display::{
    Bitmap, DisplayList, Fill, FillRule, Gradient, GradientStop, Group, Image, ImageStore, Interpolation, Node, Paint, Spread, Stroke, StrokeStyle,
};
use common::Rendered;

/// Exports one frame (white background, 100x100) and renders it. `None` when no renderer is configured.
fn render(name: &str, nodes: Vec<Node>, images: &ImageStore) -> Option<Rendered> {
    common::exporter()?;
    let list = DisplayList { size: Size::new(100.0, 100.0), background: Some(Rgba8::WHITE), nodes };
    let mut options = ExportOptions::for_list(&list, 24.0);
    options.movie.compression = Compression::None;
    let out = export_display_lists([&list], images, options).unwrap();
    let rendered = common::render_swf(name, &out.swf)?;
    assert_eq!((rendered.width, rendered.height), (100, 100));
    Some(rendered)
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> BezPath {
    Rect::new(x, y, x + w, y + h).to_path(0.1)
}

fn reversed(path: &BezPath) -> BezPath {
    // the same square walked the other way round
    let mut out = BezPath::new();
    let bounds = path.bounding_box();
    out.move_to((bounds.x0, bounds.y0));
    out.line_to((bounds.x0, bounds.y1));
    out.line_to((bounds.x1, bounds.y1));
    out.line_to((bounds.x1, bounds.y0));
    out.close_path();
    out
}

fn fill(path: BezPath, color: Rgba8, transform: Affine, rule: FillRule) -> Node {
    Node::Fill(Fill { transform, path, rule, paint: Paint::Solid(color) })
}

const RED: Rgba8 = Rgba8::opaque(255, 0, 0);
const BLUE: Rgba8 = Rgba8::opaque(0, 0, 255);
const WHITE: [u8; 3] = [255, 255, 255];

#[test]
fn solid_fills_land_where_the_transform_puts_them() {
    for (name, path) in [("clockwise", rect(0.0, 0.0, 20.0, 20.0)), ("counter_clockwise", reversed(&rect(0.0, 0.0, 20.0, 20.0)))] {
        let Some(r) = render(&format!("solid_{name}"), vec![fill(path, RED, Affine::translate((10.0, 30.0)), FillRule::NonZero)], &ImageStore::new()) else {
            return;
        };
        r.assert_near(20, 40, [255, 0, 0], 2, name);
        r.assert_near(12, 32, [255, 0, 0], 2, name);
        r.assert_near(5, 5, WHITE, 2, name);
        r.assert_near(50, 40, WHITE, 2, name);
        r.assert_near(20, 60, WHITE, 2, name);
    }
}

#[test]
fn winding_rules_and_hole_orientation() {
    let outer = rect(10.0, 10.0, 80.0, 80.0);
    let hole_same = rect(35.0, 35.0, 30.0, 30.0);
    let hole_opposite = reversed(&hole_same);
    let combine = |inner: &BezPath| {
        let mut path = outer.clone();
        path.extend(inner.clone());
        path
    };
    // even-odd: a hole regardless of direction
    for (name, inner) in [("same", &hole_same), ("opposite", &hole_opposite)] {
        let Some(r) = render(&format!("evenodd_{name}"), vec![fill(combine(inner), RED, Affine::IDENTITY, FillRule::EvenOdd)], &ImageStore::new()) else {
            return;
        };
        r.assert_near(20, 50, [255, 0, 0], 2, name);
        r.assert_near(50, 50, WHITE, 2, name);
    }
    // non-zero: only the opposite direction makes a hole
    let Some(r) = render("nonzero_opposite", vec![fill(combine(&hole_opposite), RED, Affine::IDENTITY, FillRule::NonZero)], &ImageStore::new()) else { return };
    r.assert_near(20, 50, [255, 0, 0], 2, "ring");
    r.assert_near(50, 50, WHITE, 2, "hole");
    let Some(r) = render("nonzero_same", vec![fill(combine(&hole_same), RED, Affine::IDENTITY, FillRule::NonZero)], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [255, 0, 0], 2, "same direction fills the middle");
}

#[test]
fn gradients_follow_the_flash_gradient_space() {
    // a left to right ramp over the whole stage: gradient space +-819.2 maps to x in 0..100
    let ramp = Gradient {
        transform: Affine::translate((50.0, 50.0)) * Affine::scale(50.0 / anim_display::GRADIENT_EXTENT),
        stops: vec![GradientStop { offset: 0.0, color: RED }, GradientStop { offset: 1.0, color: BLUE }],
        spread: Spread::Pad,
        interpolation: Interpolation::Rgb,
        focal: 0.0,
    };
    let linear =
        Node::Fill(Fill { transform: Affine::IDENTITY, path: rect(0.0, 0.0, 100.0, 100.0), rule: FillRule::NonZero, paint: Paint::Linear(ramp.clone()) });
    let Some(r) = render("linear", vec![linear], &ImageStore::new()) else { return };
    r.assert_near(1, 50, [253, 0, 2], 8, "left end is red");
    r.assert_near(50, 50, [127, 0, 127], 8, "middle is purple");
    r.assert_near(98, 50, [5, 0, 250], 8, "right end is blue");

    // a radial gradient of radius 40 px around (50, 50): red in the middle, blue at the rim and beyond
    let radial = Gradient { transform: Affine::translate((50.0, 50.0)) * Affine::scale(40.0 / anim_display::GRADIENT_EXTENT), ..ramp };
    let node = Node::Fill(Fill { transform: Affine::IDENTITY, path: rect(0.0, 0.0, 100.0, 100.0), rule: FillRule::NonZero, paint: Paint::Radial(radial) });
    let Some(r) = render("radial", vec![node], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [255, 0, 0], 8, "centre");
    r.assert_near(70, 50, [127, 0, 127], 12, "halfway");
    r.assert_near(95, 95, [0, 0, 255], 8, "beyond the rim");
}

#[test]
fn bitmaps_keep_their_pixels_and_orientation() {
    let mut images = ImageStore::new();
    // red green / blue yellow, premultiplied (opaque so identical)
    let id = images.insert(Image::new(2, 2, vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255]).unwrap());
    let node = Node::Bitmap(Bitmap { transform: Affine::translate((20.0, 20.0)) * Affine::scale(30.0), image: id, smoothing: false });
    let Some(r) = render("bitmap", vec![node], &images) else { return };
    r.assert_near(30, 30, [255, 0, 0], 3, "top left");
    r.assert_near(65, 30, [0, 255, 0], 3, "top right");
    r.assert_near(30, 65, [0, 0, 255], 3, "bottom left");
    r.assert_near(65, 65, [255, 255, 0], 3, "bottom right");
    r.assert_near(10, 10, WHITE, 3, "outside");
}

#[test]
fn bitmap_fills_use_the_pixel_to_local_matrix() {
    let mut images = ImageStore::new();
    let id = images.insert(Image::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap());
    // each image pixel is 20 px wide and 40 px tall, the pattern starts at (10, 10)
    let paint =
        Paint::Bitmap { image: id, transform: Affine::translate((10.0, 10.0)) * Affine::scale_non_uniform(20.0, 40.0), repeat: false, smoothing: false };
    let node = Node::Fill(Fill { transform: Affine::IDENTITY, path: rect(10.0, 10.0, 40.0, 40.0), rule: FillRule::NonZero, paint });
    let Some(r) = render("bitmap_fill", vec![node], &images) else { return };
    r.assert_near(20, 30, [255, 0, 0], 3, "first pixel");
    r.assert_near(40, 30, [0, 0, 255], 3, "second pixel");
    r.assert_near(60, 30, WHITE, 3, "outside the path");
}

#[test]
fn group_transform_colour_transform_and_masks() {
    // a red square rotated a quarter turn about (50, 50) at half opacity
    let child = fill(rect(-20.0, -10.0, 40.0, 20.0), RED, Affine::IDENTITY, FillRule::NonZero);
    let group = Node::Group(Group {
        transform: Affine::translate((50.0, 50.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2),
        color: ColorTransform::alpha(0.5),
        children: vec![child],
        ..Group::default()
    });
    let Some(r) = render("group", vec![group], &ImageStore::new()) else { return };
    // after the quarter turn the 40x20 bar is 20 wide and 40 tall
    r.assert_near(50, 35, [255, 128, 128], 4, "half transparent red over white");
    r.assert_near(50, 65, [255, 128, 128], 4, "half transparent red over white");
    r.assert_near(65, 50, WHITE, 3, "outside the rotated bar");

    // a big blue square seen through a circular mask
    let masked = Node::Group(Group {
        mask: Some(vec![fill(Circle::new((50.0, 50.0), 20.0).to_path(0.05), Rgba8::BLACK, Affine::IDENTITY, FillRule::NonZero)]),
        children: vec![fill(rect(0.0, 0.0, 100.0, 100.0), BLUE, Affine::IDENTITY, FillRule::NonZero)],
        ..Group::default()
    });
    let Some(r) = render("mask", vec![masked], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [0, 0, 255], 3, "inside the mask");
    r.assert_near(50, 35, [0, 0, 255], 3, "inside the mask");
    r.assert_near(10, 10, WHITE, 3, "outside the mask");
    r.assert_near(50, 90, WHITE, 3, "outside the mask");
}

#[test]
fn strokes_are_centred_on_the_path_with_the_right_width() {
    let mut path = BezPath::new();
    path.move_to((10.0, 50.0));
    path.line_to((90.0, 50.0));
    let stroke =
        Node::Stroke(Stroke { transform: Affine::IDENTITY, path, style: StrokeStyle { width: 10.0, ..StrokeStyle::default() }, paint: Paint::Solid(RED) });
    let Some(r) = render("stroke", vec![stroke], &ImageStore::new()) else { return };
    r.assert_near(50, 47, [255, 0, 0], 3, "above the centre line");
    r.assert_near(50, 52, [255, 0, 0], 3, "below the centre line");
    r.assert_near(50, 40, WHITE, 3, "above the stroke");
    r.assert_near(50, 60, WHITE, 3, "below the stroke");
}

#[test]
fn fill_and_stroke_of_the_same_path_share_one_shape() {
    let path = rect(20.0, 20.0, 60.0, 60.0);
    let nodes = vec![
        fill(path.clone(), RED, Affine::IDENTITY, FillRule::NonZero),
        Node::Stroke(Stroke { transform: Affine::IDENTITY, path, style: StrokeStyle { width: 8.0, ..StrokeStyle::default() }, paint: Paint::Solid(BLUE) }),
    ];
    let Some(r) = render("fill_stroke", nodes, &ImageStore::new()) else { return };
    r.assert_near(50, 50, [255, 0, 0], 3, "fill");
    r.assert_near(20, 50, [0, 0, 255], 3, "stroke over the fill's edge");
    r.assert_near(10, 50, WHITE, 3, "outside");
}
