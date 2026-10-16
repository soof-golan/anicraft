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

use crate::common;

use anicraft_swf::export::{ExportOptions, export_display_lists};
use anicraft_swf::movie::Compression;
use anim_core::kurbo::{BezPath, Circle, Rect, Shape};
use anim_core::{Affine, BlendMode, ColorTransform, Rgba8, Size};
use anim_display::{
    Bitmap, DisplayList, Fill, FillRule, Filter, Gradient, GradientStop, Group, Image, ImageStore, Interpolation, Node, Paint, Spread, Stroke, StrokeStyle,
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

fn group_of(child: Node, blend: BlendMode, filters: Vec<Filter>) -> Node {
    Node::Group(Group { blend, filters, children: vec![child], ..Group::default() })
}

#[test]
fn blend_modes_combine_with_what_is_below() {
    let backdrop = fill(rect(0.0, 0.0, 100.0, 100.0), Rgba8::opaque(200, 100, 50), Affine::IDENTITY, FillRule::NonZero);
    let top = |mode| group_of(fill(rect(20.0, 20.0, 60.0, 60.0), Rgba8::opaque(100, 200, 255), Affine::IDENTITY, FillRule::NonZero), mode, vec![]);
    // multiply: channel products
    let Some(r) = render("blend_multiply", vec![backdrop.clone(), top(BlendMode::Multiply)], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [78, 78, 50], 4, "multiply");
    r.assert_near(5, 5, [200, 100, 50], 2, "outside the blended square");
    // screen: 255 - (255 - a)(255 - b) / 255
    let Some(r) = render("blend_screen", vec![backdrop.clone(), top(BlendMode::Screen)], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [222, 222, 255], 4, "screen");
    // difference
    let Some(r) = render("blend_difference", vec![backdrop.clone(), top(BlendMode::Difference)], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [100, 100, 205], 4, "difference");
    // add saturates
    let Some(r) = render("blend_add", vec![backdrop, top(BlendMode::Add)], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [255, 255, 255], 4, "add");
}

#[test]
fn filters_use_flash_units_and_directions() {
    let square = || fill(rect(40.0, 40.0, 20.0, 20.0), RED, Affine::IDENTITY, FillRule::NonZero);
    // a hard shadow straight down: angle 90 degrees, 12 px away
    let shadow = Filter::DropShadow {
        color: Rgba8::BLACK,
        angle_deg: 90.0,
        distance: 12.0,
        blur_x: 1.0,
        blur_y: 1.0,
        strength: 1.0,
        inner: false,
        knockout: false,
        hide_object: false,
    };
    let Some(r) = render("filter_shadow", vec![group_of(square(), BlendMode::Normal, vec![shadow])], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [255, 0, 0], 3, "the object stays on top");
    r.assert_near(50, 68, [0, 0, 0], 40, "the shadow is below the object");
    r.assert_near(50, 30, WHITE, 5, "nothing above it");
    r.assert_near(70, 50, WHITE, 5, "nothing beside it");

    // a wide blur spreads colour out of the square and thins it in the middle
    let blur = Filter::Blur { x: 20.0, y: 20.0, quality: 2 };
    let Some(r) = render("filter_blur", vec![group_of(square(), BlendMode::Normal, vec![blur])], &ImageStore::new()) else { return };
    let outside = (30..40).map(|x| r.at(x, 50)).min_by_key(|p| p[1]).unwrap();
    assert!(outside[1] < 255 && outside[0] > 200, "blur reaches outside the square: {outside:?}");
    let middle = r.at(50, 50);
    assert!(middle[1] > 20, "the middle is no longer pure red: {middle:?}");
    r.assert_near(5, 5, WHITE, 3, "far away is untouched");

    // a colour matrix that swaps red and blue
    let mut swap = [0.0_f32; 20];
    swap[2] = 1.0; // r' = b
    swap[6] = 1.0; // g' = g
    swap[10] = 1.0; // b' = r
    swap[18] = 1.0; // a' = a
    let Some(r) = render("filter_matrix", vec![group_of(square(), BlendMode::Normal, vec![Filter::ColorMatrix(swap)])], &ImageStore::new()) else { return };
    r.assert_near(50, 50, [0, 0, 255], 3, "red became blue");
}

#[test]
fn colour_transform_offsets_and_multipliers() {
    // brighten a dark square with an additive offset and halve the green
    let child = fill(rect(20.0, 20.0, 60.0, 60.0), Rgba8::opaque(100, 200, 50), Affine::IDENTITY, FillRule::NonZero);
    let group =
        Node::Group(Group { color: ColorTransform { mul: [1.0, 0.5, 1.0, 1.0], add: [0.2, 0.0, 0.0, 0.0] }, children: vec![child], ..Group::default() });
    let Some(r) = render("colour_transform", vec![group], &ImageStore::new()) else { return };
    // red: 100 + 51, green: 100, blue: 50
    r.assert_near(50, 50, [151, 100, 50], 4, "multiplied and offset");
}
