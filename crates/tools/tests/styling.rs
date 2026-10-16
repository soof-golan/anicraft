//! Tool options and colour: brush modes and tips, eraser tips and faucet, snapping, fitted gradients and the
//! Gradient Transform tool.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::float_cmp,
    clippy::assert_is_empty,
    clippy::manual_assert_eq,
    clippy::suboptimal_flops
)]

mod common;

use anicraft_tools::{BrushMode, EraserMode, TipShape, ToolId};
use anim_core::{Affine, Point, Rgba8};
use anim_display::{GradientStop, Interpolation, Spread};
use anim_shape::{FillStyle, GradientFill};
use common::*;

fn two_stop_gradient() -> GradientFill {
    GradientFill {
        matrix: Affine::IDENTITY,
        stops: vec![GradientStop { offset: 0.0, color: Rgba8::BLACK }, GradientStop { offset: 1.0, color: Rgba8::opaque(255, 255, 255) }],
        spread: Spread::Pad,
        interpolation: Interpolation::Rgb,
        focal: 0.0,
    }
}

fn gradient_matrix(e: &anicraft_tools::Editor, index: usize) -> Affine {
    match shape_at(e, index).shape.fills.first() {
        Some(FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g)) => g.matrix,
        other => panic!("not a gradient: {other:?}"),
    }
}

#[test]
fn a_gradient_fill_is_fitted_to_the_rectangle_it_paints() {
    let mut e = editor();
    e.state.settings.fill = FillStyle::LinearGradient(two_stop_gradient());
    drag(&mut e, ToolId::Rectangle, (100.0, 50.0), (300.0, 150.0));
    let m = gradient_matrix(&e, 0);
    let left = m * Point::new(-819.2, 0.0);
    let right = m * Point::new(819.2, 0.0);
    assert!((left.x - 100.0).abs() < 0.2 && (right.x - 300.0).abs() < 0.2, "{left:?} {right:?}");
}

#[test]
fn bucket_fits_the_gradient_to_the_area_unless_the_fill_is_locked() {
    for lock in [false, true] {
        let mut e = editor();
        e.state.settings.fill_enabled = false;
        drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (200.0, 160.0));
        e.state.settings.fill_enabled = true;
        e.state.settings.fill = FillStyle::LinearGradient(two_stop_gradient());
        e.state.settings.bucket_lock_fill = lock;
        click(&mut e, ToolId::PaintBucket, (150.0, 130.0));
        let shape = &shape_at(&e, 0).shape;
        let Some(FillStyle::LinearGradient(g)) = shape.fills.first() else { panic!("gradient fill expected: {:?}", shape.fills) };
        let right = g.matrix * Point::new(819.2, 0.0);
        if lock {
            assert_eq!(g.matrix, Affine::IDENTITY);
        } else {
            assert!((right.x - 200.0).abs() < 0.5, "fitted to the clicked area: {right:?}");
        }
    }
}

fn paint_stroke(e: &mut anicraft_tools::Editor, from: (f64, f64), to: (f64, f64)) {
    drag(e, ToolId::Brush, from, to);
}

#[test]
fn brush_tip_shape_and_mode_change_what_is_painted() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    e.state.settings.brush_size = 20.0;
    e.state.settings.brush_pressure = false;
    e.state.settings.brush_smoothing = 0.0;
    e.state.settings.brush_shape = TipShape::Square;
    paint_stroke(&mut e, (100.0, 100.0), (200.0, 100.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(92.0, 92.0)).is_some(), "a square tip reaches the corner");
    assert!(shape.fill_at(Point::new(150.0, 120.0)).is_none());

    // Paint behind: the new stroke crosses the old one but does not cover it.
    e.state.settings.brush_shape = TipShape::Round;
    e.state.settings.brush_mode = BrushMode::PaintBehind;
    e.state.settings.fill = FillStyle::Solid(BLUE);
    paint_stroke(&mut e, (150.0, 60.0), (150.0, 140.0));
    let shape = &shape_at(&e, 0).shape;
    assert_eq!(fill_color_at(shape, Point::new(150.0, 100.0)), Some(RED), "behind leaves the old paint on top");
    assert_eq!(fill_color_at(shape, Point::new(150.0, 70.0)), Some(BLUE));

    // Paint inside: only where the press started in a fill.
    e.state.settings.brush_mode = BrushMode::PaintInside;
    e.state.settings.fill = FillStyle::Solid(Rgba8::opaque(0, 255, 0));
    paint_stroke(&mut e, (120.0, 100.0), (260.0, 100.0));
    let shape = &shape_at(&e, 0).shape;
    assert_eq!(fill_color_at(shape, Point::new(130.0, 100.0)), Some(Rgba8::opaque(0, 255, 0)));
    assert!(shape.fill_at(Point::new(250.0, 100.0)).is_none(), "nothing outside the fill under the press");
}

#[test]
fn eraser_tip_and_faucet() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (200.0, 200.0));
    e.state.settings.eraser_size = 20.0;
    e.state.settings.eraser_shape = TipShape::Square;
    click(&mut e, ToolId::Eraser, (150.0, 150.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(158.0, 158.0)).is_none(), "the square tip erased the corner of its box");
    assert!(shape.fill_at(Point::new(120.0, 120.0)).is_some());

    e.state.settings.eraser_faucet = true;
    e.state.settings.eraser_mode = EraserMode::Normal;
    click(&mut e, ToolId::Eraser, (120.0, 120.0));
    assert!(elements(&e).is_empty() || shape_at(&e, 0).shape.fill_at(Point::new(120.0, 120.0)).is_none(), "the faucet removed the whole fill");
}

#[test]
fn snapping_moves_drawn_corners_to_the_grid() {
    let mut e = editor();
    e.state.settings.snap.grid = true;
    e.state.settings.snap.grid_size = 25.0;
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (52.0, 48.0), (148.0, 103.0));
    let b = shape_at(&e, 0).shape.bounds().unwrap();
    assert!((b.x0 - 50.0).abs() < 0.1 && (b.y0 - 50.0).abs() < 0.1, "{b:?}");
    assert!((b.x1 - 150.0).abs() < 0.1 && (b.y1 - 100.0).abs() < 0.1, "{b:?}");
}

#[test]
fn snapping_to_objects_lands_on_an_anchor() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (200.0, 200.0));
    e.state.settings.snap.objects = true;
    e.state.settings.object_drawing = true;
    e.state.settings.stroke_enabled = true;
    drag(&mut e, ToolId::Line, (203.0, 197.0), (300.0, 300.0));
    let b = shape_at(&e, 1).shape.bounds().unwrap();
    assert!((b.x0 - 200.0).abs() < 0.2 && (b.y0 - 200.0).abs() < 0.2, "the line starts on the corner of the rectangle: {b:?}");
}

#[test]
fn gradient_transform_moves_scales_and_rotates_the_fill() {
    let mut e = editor();
    e.state.settings.fill = FillStyle::LinearGradient(two_stop_gradient());
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    let before = gradient_matrix(&e, 0);
    e.select_tool(ToolId::GradientTransform);
    // Show the handles by clicking the fill, then drag the centre 30 px to the right.
    send_all(&mut e, vec![down(150.0, 150.0), up(150.0, 150.0)]);
    let overlay = e.overlay();
    assert!(!overlay.is_empty(), "handles are shown");
    send_all(&mut e, vec![down(200.0, 150.0), mv(215.0, 150.0), mv(230.0, 150.0), up(230.0, 150.0)]);
    let moved = gradient_matrix(&e, 0);
    assert!(((moved * Point::ZERO).x - (before * Point::ZERO).x - 30.0).abs() < 0.5);

    // The square handle sits at the right end of the gradient: dragging it doubles the width.
    let right = moved * Point::new(819.2, 0.0);
    let centre = moved * Point::ZERO;
    let target = Point::new(centre.x + (right.x - centre.x) * 2.0, right.y);
    send_all(&mut e, vec![down(right.x, right.y), mv(target.x, target.y), up(target.x, target.y)]);
    let scaled = gradient_matrix(&e, 0);
    let new_right = scaled * Point::new(819.2, 0.0);
    assert!(((new_right.x - centre.x) - (right.x - centre.x) * 2.0).abs() < 0.5, "{new_right:?}");

    // The circle above the centre rotates the fill a quarter turn.
    let handle = scaled * Point::new(0.0, -819.2);
    let c = scaled * Point::ZERO;
    let end = Point::new(c.x + (c.y - handle.y), c.y + (handle.x - c.x));
    send_all(&mut e, vec![down(handle.x, handle.y), mv(end.x, end.y), up(end.x, end.y)]);
    let rotated = gradient_matrix(&e, 0);
    let axis = rotated * Point::new(819.2, 0.0) - rotated * Point::ZERO;
    assert!(axis.x.abs() < 1.0 && axis.y.abs() > 100.0, "the gradient line now runs vertically: {axis:?}");
}

#[test]
fn gradient_transform_ignores_solid_fills() {
    let mut e = editor();
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    e.select_tool(ToolId::GradientTransform);
    send_all(&mut e, vec![down(150.0, 150.0), up(150.0, 150.0)]);
    assert!(e.overlay().is_empty());
}
