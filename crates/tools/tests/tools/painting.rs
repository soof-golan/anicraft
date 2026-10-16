//! Paint Bucket, Ink Bottle, Eraser and Eyedropper.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::float_cmp,
    clippy::manual_assert_eq,
    clippy::assert_is_empty,
    clippy::suboptimal_flops,
    clippy::many_single_char_names
)]

use crate::common;

use anicraft_tools::{EraserMode, Modifiers, PointerEvent, ToolCommand, ToolId};
use anim_core::{Affine, BezPath, Point, Rgba8, Vec2};
use anim_scene::{Element, ShapeElement, TimelineId, edit};
use anim_shape::{EdgeShape, FillStyle, Hit, PathInput, StrokeStyle};
use common::*;

fn outline_only(editor: &mut anicraft_tools::Editor) {
    let shape = rect_shape(100.0, 100.0, 300.0, 200.0, None, Some(2.0));
    edit::add_element(&mut editor.document, &TimelineId::Scene(0), 0, 0, Element::Shape(ShapeElement::new(shape))).unwrap();
}

#[test]
fn bucket_fills_a_closed_outline() {
    let mut e = editor();
    outline_only(&mut e);
    e.state.settings.fill = FillStyle::Solid(BLUE);
    let commands = click(&mut e, ToolId::PaintBucket, (200.0, 150.0));
    assert!(matches!(commands.as_slice(), [ToolCommand::ReplaceShape { .. }]));
    let shape = &shape_at(&e, 0).shape;
    assert_eq!(fill_color_at(shape, Point::new(200.0, 150.0)), Some(BLUE));
    assert!(matches!(shape.hit_test(Point::new(100.0, 150.0), 1.0), Some(Hit::Stroke { .. })), "the outline survives");
    assert!(shape.fill_at(Point::new(400.0, 150.0)).is_none());
}

#[test]
fn bucket_outside_any_area_does_nothing() {
    let mut e = editor();
    outline_only(&mut e);
    assert!(click(&mut e, ToolId::PaintBucket, (50.0, 50.0)).is_empty());
}

#[test]
fn bucket_recolours_an_existing_fill() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    e.state.settings.fill = FillStyle::Solid(BLUE);
    click(&mut e, ToolId::PaintBucket, (150.0, 150.0));
    let shape = &shape_at(&e, 0).shape;
    assert_eq!(fill_color_at(shape, Point::new(150.0, 150.0)), Some(BLUE));
    assert_eq!(elements(&e).len(), 1);
    assert!(e.undo());
    assert_eq!(fill_color_at(&shape_at(&e, 0).shape, Point::new(150.0, 150.0)), Some(RED));
}

#[test]
fn bucket_gap_option_closes_small_gaps() {
    let mut e = editor();
    let mut p = BezPath::new();
    p.move_to((100.0, 100.0));
    p.line_to((300.0, 100.0));
    p.line_to((300.0, 200.0));
    p.line_to((100.0, 200.0));
    p.line_to((100.0, 106.0));
    let shape = EdgeShape::from_paths(&[PathInput::new(p, None, Some(StrokeStyle::plain(2.0, Rgba8::BLACK)))]);
    edit::add_element(&mut e.document, &TimelineId::Scene(0), 0, 0, Element::Shape(ShapeElement::new(shape))).unwrap();
    assert!(click(&mut e, ToolId::PaintBucket, (200.0, 150.0)).is_empty(), "open outline: no fill");
    e.state.settings.bucket_gap = anicraft_tools::GapSize::Medium;
    assert_eq!(click(&mut e, ToolId::PaintBucket, (200.0, 150.0)).len(), 1);
}

#[test]
fn bucket_works_inside_a_transformed_shape() {
    let mut e = editor();
    let mut element = ShapeElement::new(rect_shape(0.0, 0.0, 100.0, 50.0, None, Some(1.0)));
    element.matrix = Affine::translate(Vec2::new(200.0, 100.0)) * Affine::scale(2.0);
    edit::add_element(&mut e.document, &TimelineId::Scene(0), 0, 0, Element::Shape(element)).unwrap();
    click(&mut e, ToolId::PaintBucket, (300.0, 150.0));
    assert!(fill_color_at(&shape_at(&e, 0).shape, Point::new(50.0, 25.0)).is_some(), "the click was mapped into the shape's own space");
}

#[test]
fn ink_bottle_restyles_a_stroke_and_outlines_a_fill() {
    let mut e = editor();
    outline_only(&mut e);
    e.state.settings.stroke = StrokeStyle::plain(6.0, BLUE);
    let commands = click(&mut e, ToolId::InkBottle, (100.0, 150.0));
    assert!(matches!(commands.as_slice(), [ToolCommand::ReplaceShape { .. }]));
    assert!(shape_at(&e, 0).shape.strokes.contains(&StrokeStyle::plain(6.0, BLUE)));

    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    e.state.settings.stroke = StrokeStyle::plain(4.0, Rgba8::opaque(0, 0, 0));
    click(&mut e, ToolId::InkBottle, (200.0, 150.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(matches!(shape.hit_test(Point::new(100.0, 150.0), 1.0), Some(Hit::Stroke { .. })));
    assert_eq!(fill_color_at(shape, Point::new(200.0, 150.0)), Some(RED));
    assert!(click(&mut e, ToolId::InkBottle, (450.0, 350.0)).is_empty());
}

#[test]
fn eraser_cuts_a_channel_through_a_shape() {
    let mut e = editor();
    e.state.settings.eraser_size = 20.0;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    let commands = drag(&mut e, ToolId::Eraser, (80.0, 150.0), (320.0, 150.0));
    assert!(matches!(commands.as_slice(), [ToolCommand::ReplaceShape { .. }]), "{commands:?}");
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(200.0, 150.0)).is_none());
    assert!(shape.fill_at(Point::new(200.0, 120.0)).is_some());
    assert!(shape.fill_at(Point::new(200.0, 180.0)).is_some());
    assert!(shape.hit_test(Point::new(100.0, 150.0), 1.0).is_none(), "the stroke is cut as well");
    assert!(shape.hit_test(Point::new(100.0, 110.0), 1.0).is_some());
}

#[test]
fn eraser_modes_limit_what_is_removed() {
    let mut e = editor();
    e.state.settings.eraser_size = 20.0;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    e.state.settings.eraser_mode = EraserMode::Lines;
    drag(&mut e, ToolId::Eraser, (200.0, 150.0), (210.0, 150.0));
    assert!(shape_at(&e, 0).shape.fill_at(Point::new(200.0, 150.0)).is_some(), "lines mode leaves the fill alone");
    e.state.settings.eraser_mode = EraserMode::Fills;
    drag(&mut e, ToolId::Eraser, (95.0, 150.0), (110.0, 150.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(105.0, 150.0)).is_none());
    assert!(shape.hit_test(Point::new(100.0, 150.0), 1.0).is_some(), "fills mode leaves the stroke alone");
}

#[test]
fn erasing_everything_removes_the_element() {
    let mut e = editor();
    e.state.settings.eraser_size = 400.0;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (150.0, 140.0));
    let commands = drag(&mut e, ToolId::Eraser, (120.0, 120.0), (130.0, 120.0));
    assert!(matches!(commands.as_slice(), [ToolCommand::DeleteElements { .. }]), "{commands:?}");
    assert!(elements(&e).is_empty());
}

#[test]
fn eraser_misses_leave_the_document_alone() {
    let mut e = editor();
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (150.0, 140.0));
    let before = e.undo_depth();
    assert!(drag(&mut e, ToolId::Eraser, (400.0, 300.0), (450.0, 350.0)).is_empty());
    assert_eq!(e.undo_depth(), before);
}

#[test]
fn eraser_respects_object_transforms() {
    let mut e = editor();
    e.state.settings.eraser_size = 10.0;
    let mut element = ShapeElement::new(rect_shape(0.0, 0.0, 100.0, 100.0, Some(RED), None));
    element.matrix = Affine::translate(Vec2::new(200.0, 200.0)) * Affine::scale(2.0);
    edit::add_element(&mut e.document, &TimelineId::Scene(0), 0, 0, Element::Shape(element)).unwrap();
    // Stage point (300, 300) is shape point (50, 50).
    drag(&mut e, ToolId::Eraser, (300.0, 300.0), (310.0, 300.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(50.0, 50.0)).is_none());
    assert!(shape.fill_at(Point::new(10.0, 10.0)).is_some());
}

#[test]
fn eyedropper_loads_styles_and_switches_to_the_matching_tool() {
    let mut e = editor();
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    e.state.settings.fill = FillStyle::Solid(BLUE);
    e.state.settings.stroke = StrokeStyle::plain(9.0, BLUE);
    let commands = click(&mut e, ToolId::Eyedropper, (200.0, 150.0));
    assert_eq!(
        commands,
        vec![ToolCommand::SetToolStyle { fill: Some(FillStyle::Solid(RED)), stroke: None }, ToolCommand::ActivateTool { tool: ToolId::PaintBucket }]
    );
    assert_eq!(e.state.settings.fill, FillStyle::Solid(RED));
    assert_eq!(e.tools.active(), ToolId::PaintBucket);

    let commands = click(&mut e, ToolId::Eyedropper, (100.0, 150.0));
    assert!(matches!(commands.as_slice(), [ToolCommand::SetToolStyle { fill: None, stroke: Some(_) }, ToolCommand::ActivateTool { tool: ToolId::InkBottle }]));
    assert_eq!(e.state.settings.stroke, StrokeStyle::plain(1.0, Rgba8::BLACK));
    assert_eq!(e.tools.active(), ToolId::InkBottle);
    assert!(click(&mut e, ToolId::Eyedropper, (450.0, 350.0)).is_empty());
}

#[test]
fn modifiers_do_not_break_painting_tools() {
    let mut e = editor();
    outline_only(&mut e);
    e.select_tool(ToolId::PaintBucket);
    let shift = Modifiers::SHIFT;
    let cmds =
        send_all(&mut e, vec![PointerEvent::down(200.0, 150.0).with_modifiers(shift).into(), PointerEvent::up(200.0, 150.0).with_modifiers(shift).into()]);
    assert_eq!(cmds.len(), 1);
}
