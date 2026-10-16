//! Line, Rectangle, Oval, Pen, Pencil and Brush: an event script in, the expected commands and shapes out.

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

use anicraft_tools::{Key, KeyEvent, Modifiers, PointerEvent, ToolCommand, ToolEvent, ToolId};
use anim_core::{Point, Rgba8};
use anim_scene::Element;
use anim_shape::{FillStyle, Hit, Segment};
use common::*;

fn single_draw(commands: &[ToolCommand]) -> (&anim_shape::EdgeShape, bool) {
    let draws: Vec<_> = commands
        .iter()
        .filter_map(|c| if let ToolCommand::DrawShape { shape, object_drawing, .. } = c { Some((shape, *object_drawing)) } else { None })
        .collect();
    assert_eq!(draws.len(), 1, "exactly one DrawShape in {commands:?}");
    draws[0]
}

#[test]
fn rectangle_drag_draws_a_filled_stroked_rectangle() {
    let mut e = editor();
    let commands = drag(&mut e, ToolId::Rectangle, (50.0, 40.0), (250.0, 140.0));
    let (shape, object) = single_draw(&commands);
    assert!(!object);
    assert_eq!(fill_color_at(shape, Point::new(150.0, 90.0)), Some(RED));
    assert!(shape.fill_at(Point::new(300.0, 90.0)).is_none());
    assert!(matches!(shape.hit_test(Point::new(50.0, 90.0), 1.0), Some(Hit::Stroke { .. })));
    // The command was applied: the merge shape is on the stage.
    assert_eq!(elements(&e).len(), 1);
    assert_eq!(fill_color_at(&shape_at(&e, 0).shape, Point::new(150.0, 90.0)), Some(RED));
    let b = shape_at(&e, 0).shape.bounds().unwrap();
    assert!((b.x0 - 50.0).abs() < 0.1 && (b.y1 - 140.0).abs() < 0.1);
}

#[test]
fn rectangle_corner_radius_rounds_the_corners() {
    let mut e = editor();
    e.state.settings.corner_radius = 30.0;
    drag(&mut e, ToolId::Rectangle, (50.0, 50.0), (250.0, 150.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(52.0, 52.0)).is_none(), "the corner is cut away");
    assert!(shape.fill_at(Point::new(80.0, 80.0)).is_some());
    assert!(shape.fill_at(Point::new(150.0, 52.0)).is_some());
}

#[test]
fn unlocked_corner_radii_round_only_their_corners() {
    let mut e = editor();
    e.state.settings.corner_lock = false;
    e.state.settings.corner_radii = [40.0, 0.0, 0.0, 0.0];
    drag(&mut e, ToolId::Rectangle, (50.0, 50.0), (250.0, 150.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(52.0, 52.0)).is_none(), "the top left corner is cut away");
    assert!(shape.fill_at(Point::new(248.0, 52.0)).is_some(), "the top right corner stays square");
    assert!(shape.fill_at(Point::new(52.0, 148.0)).is_some(), "the bottom left corner stays square");
}

#[test]
fn oval_arc_settings_draw_pies_and_rings() {
    let mut e = editor();
    e.state.settings.oval = anicraft_tools::settings::OvalSettings { start_angle: 0.0, end_angle: 90.0, inner_radius: 0.0, close_path: true };
    drag(&mut e, ToolId::Oval, (100.0, 100.0), (300.0, 300.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(250.0, 250.0)).is_some(), "the quarter between three and six o'clock is filled");
    assert!(shape.fill_at(Point::new(150.0, 150.0)).is_none(), "the opposite quarter is empty");

    let mut e = editor();
    e.state.settings.oval = anicraft_tools::settings::OvalSettings { start_angle: 0.0, end_angle: 0.0, inner_radius: 50.0, close_path: true };
    drag(&mut e, ToolId::Oval, (100.0, 100.0), (300.0, 300.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(200.0, 200.0)).is_none(), "the hole is empty");
    assert!(shape.fill_at(Point::new(200.0, 120.0)).is_some(), "the ring is filled");
}

#[test]
fn shift_and_alt_constrain_the_rectangle() {
    let mut e = editor();
    e.select_tool(ToolId::Rectangle);
    let shift = Modifiers::SHIFT;
    let events: Vec<ToolEvent> = vec![
        PointerEvent::down(100.0, 100.0).with_modifiers(shift).into(),
        PointerEvent::drag(180.0, 140.0).with_modifiers(shift).into(),
        PointerEvent::up(180.0, 140.0).with_modifiers(shift).into(),
    ];
    send_all(&mut e, events);
    let b = shape_at(&e, 0).shape.bounds().unwrap();
    assert!((b.width() - b.height()).abs() < 0.1, "square: {b:?}");
    assert!((b.width() - 80.0).abs() < 0.1);

    let mut e = editor();
    e.select_tool(ToolId::Rectangle);
    let events: Vec<ToolEvent> =
        vec![PointerEvent::down(200.0, 200.0).with_modifiers(Modifiers::ALT).into(), PointerEvent::up(230.0, 220.0).with_modifiers(Modifiers::ALT).into()];
    send_all(&mut e, events);
    let b = shape_at(&e, 0).shape.bounds().unwrap();
    assert!((b.center().x - 200.0).abs() < 0.1 && (b.center().y - 200.0).abs() < 0.1, "centred on the press: {b:?}");
}

#[test]
fn object_drawing_creates_a_separate_selected_object() {
    let mut e = editor();
    e.state.settings.object_drawing = true;
    drag(&mut e, ToolId::Rectangle, (10.0, 10.0), (110.0, 60.0));
    drag(&mut e, ToolId::Oval, (60.0, 30.0), (160.0, 90.0));
    assert_eq!(elements(&e).len(), 2);
    assert!(shape_at(&e, 0).is_drawing_object && shape_at(&e, 1).is_drawing_object);
    assert!(shape_at(&e, 0).transformation_point.is_some());
    assert_eq!(e.state.selection.items.len(), 1);
    assert_eq!(e.state.selection.items[0].path, vec![1]);
}

#[test]
fn merge_mode_cuts_the_shape_underneath() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (50.0, 50.0), (150.0, 150.0));
    e.state.settings.fill = FillStyle::Solid(BLUE);
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (200.0, 200.0));
    assert_eq!(elements(&e).len(), 1, "merge mode keeps one shape per layer");
    let shape = &shape_at(&e, 0).shape;
    assert_eq!(fill_color_at(shape, Point::new(70.0, 70.0)), Some(RED));
    assert_eq!(fill_color_at(shape, Point::new(120.0, 120.0)), Some(BLUE));
    assert_eq!(fill_color_at(shape, Point::new(180.0, 180.0)), Some(BLUE));
    assert!(shape.fill_at(Point::new(180.0, 70.0)).is_none());
}

#[test]
fn oval_is_round_and_shift_makes_a_circle() {
    let mut e = editor();
    e.select_tool(ToolId::Oval);
    let events: Vec<ToolEvent> =
        vec![PointerEvent::down(100.0, 100.0).with_modifiers(Modifiers::SHIFT).into(), PointerEvent::up(300.0, 160.0).with_modifiers(Modifiers::SHIFT).into()];
    send_all(&mut e, events);
    let shape = &shape_at(&e, 0).shape;
    let b = shape.bounds().unwrap();
    assert!((b.width() - b.height()).abs() < 0.2, "circle: {b:?}");
    assert!(shape.fill_at(b.center()).is_some());
    assert!(shape.fill_at(Point::new(b.x0 + 2.0, b.y0 + 2.0)).is_none(), "corners of the bounding box are outside");
}

#[test]
fn line_has_a_stroke_and_no_fill_and_snaps_with_shift() {
    let mut e = editor();
    let commands = drag(&mut e, ToolId::Line, (20.0, 20.0), (220.0, 20.0));
    let (shape, _) = single_draw(&commands);
    assert!(shape.fills.is_empty());
    assert_eq!(shape.strokes.len(), 1);
    assert!(matches!(shape.hit_test(Point::new(120.0, 20.0), 1.0), Some(Hit::Stroke { .. })));

    let mut e = editor();
    e.select_tool(ToolId::Line);
    let events: Vec<ToolEvent> = vec![
        PointerEvent::down(0.0, 0.0).into(),
        PointerEvent::drag(100.0, 8.0).with_modifiers(Modifiers::SHIFT).into(),
        PointerEvent::up(100.0, 8.0).with_modifiers(Modifiers::SHIFT).into(),
    ];
    send_all(&mut e, events);
    let b = shape_at(&e, 0).shape.bounds().unwrap();
    assert!(b.height() < 0.1, "horizontal after snapping: {b:?}");
}

#[test]
fn tiny_drags_and_escape_draw_nothing() {
    let mut e = editor();
    assert!(click(&mut e, ToolId::Rectangle, (10.0, 10.0)).is_empty());
    e.select_tool(ToolId::Oval);
    let cmds = send_all(&mut e, vec![down(10.0, 10.0), mv(80.0, 80.0), KeyEvent::press(Key::Escape).into(), up(80.0, 80.0)]);
    assert!(cmds.is_empty());
    assert!(elements(&e).is_empty());
}

#[test]
fn locked_layers_are_not_drawn_on() {
    let mut e = editor();
    e.document.timeline_mut(&anim_scene::TimelineId::Scene(0)).unwrap().layers[0].locked = true;
    assert!(drag(&mut e, ToolId::Rectangle, (0.0, 0.0), (50.0, 50.0)).is_empty());
}

#[test]
fn pen_clicks_make_a_closed_filled_polygon() {
    let mut e = editor();
    e.select_tool(ToolId::Pen);
    let events =
        vec![down(50.0, 50.0), up(50.0, 50.0), down(200.0, 50.0), up(200.0, 50.0), down(125.0, 160.0), up(125.0, 160.0), down(50.0, 50.0), up(50.0, 50.0)];
    let commands = send_all(&mut e, events);
    let (shape, _) = single_draw(&commands);
    assert_eq!(fill_color_at(shape, Point::new(125.0, 80.0)), Some(RED));
    assert!(shape.fill_at(Point::new(60.0, 150.0)).is_none());
    assert_eq!(elements(&e).len(), 1);
}

#[test]
fn pen_drag_makes_smooth_curves() {
    let mut e = editor();
    e.select_tool(ToolId::Pen);
    e.state.settings.fill_enabled = false;
    let events =
        vec![down(50.0, 100.0), mv(80.0, 40.0), up(80.0, 40.0), down(250.0, 100.0), mv(280.0, 160.0), up(280.0, 160.0), KeyEvent::press(Key::Enter).into()];
    let commands = send_all(&mut e, events);
    let (shape, _) = single_draw(&commands);
    let has_curve = shape.edges.iter().flat_map(|edge| edge.segments.iter()).any(|s| !matches!(s, Segment::Line { .. }));
    assert!(has_curve, "dragging out handles curves the segment");
    assert!(shape.fills.is_empty(), "open path without fill");
    let b = shape.bounds().unwrap();
    assert!(b.y0 < 100.0 - 5.0, "the curve bulges above the anchors: {b:?}");
}

#[test]
fn pen_double_click_finishes_and_escape_cancels() {
    let mut e = editor();
    e.select_tool(ToolId::Pen);
    e.state.settings.fill_enabled = false;
    let commands = send_all(
        &mut e,
        vec![down(10.0, 10.0), up(10.0, 10.0), down(100.0, 10.0), up(100.0, 10.0), PointerEvent::down(100.0, 10.0).with_click_count(2).into(), up(100.0, 10.0)],
    );
    let (shape, _) = single_draw(&commands);
    assert!(shape.bounds().unwrap().width() > 89.0);

    let mut e = editor();
    e.select_tool(ToolId::Pen);
    let commands = send_all(
        &mut e,
        vec![down(10.0, 10.0), up(10.0, 10.0), down(100.0, 10.0), up(100.0, 10.0), KeyEvent::press(Key::Escape).into(), KeyEvent::press(Key::Enter).into()],
    );
    assert!(commands.is_empty());
    assert!(elements(&e).is_empty());
}

#[test]
fn pen_backspace_removes_the_last_point() {
    let mut e = editor();
    e.select_tool(ToolId::Pen);
    e.state.settings.fill_enabled = false;
    let commands = send_all(
        &mut e,
        vec![
            down(0.0, 0.0),
            up(0.0, 0.0),
            down(100.0, 0.0),
            up(100.0, 0.0),
            down(100.0, 300.0),
            up(100.0, 300.0),
            KeyEvent::press(Key::Backspace).into(),
            KeyEvent::press(Key::Enter).into(),
        ],
    );
    let (shape, _) = single_draw(&commands);
    assert!(shape.bounds().unwrap().height() < 1.0);
}

#[test]
fn pencil_smooths_a_wavy_stroke_into_few_curves() {
    let mut e = editor();
    e.select_tool(ToolId::Pencil);
    let points: Vec<Point> = (0..=120).map(|i| Point::new(20.0 + f64::from(i) * 3.0, 100.0 + (f64::from(i) * 0.15).sin() * 40.0)).collect();
    let mut events: Vec<ToolEvent> = vec![down(points[0].x, points[0].y)];
    events.extend(points[1..points.len() - 1].iter().map(|p| mv(p.x, p.y)));
    let last = points[points.len() - 1];
    events.push(up(last.x, last.y));
    let commands = send_all(&mut e, events);
    let (shape, _) = single_draw(&commands);
    let segments: usize = shape.edges.iter().map(|edge| edge.segments.len()).sum();
    assert!(segments < 30, "{segments} segments for 120 samples");
    assert!(shape.fills.is_empty());
    assert!(matches!(shape.hit_test(Point::new(20.0, 100.0), 2.0), Some(Hit::Stroke { .. })));
}

#[test]
fn pencil_loop_closes_and_fills() {
    let mut e = editor();
    e.select_tool(ToolId::Pencil);
    let mut events: Vec<ToolEvent> = Vec::new();
    for i in 0..=40 {
        let a = f64::from(i) / 40.0 * std::f64::consts::TAU;
        let (x, y) = (200.0 + 60.0 * a.cos(), 150.0 + 60.0 * a.sin());
        events.push(if i == 0 {
            down(x, y)
        } else if i == 40 {
            up(x + 1.0, y)
        } else {
            mv(x, y)
        });
    }
    let commands = send_all(&mut e, events);
    let (shape, _) = single_draw(&commands);
    assert_eq!(fill_color_at(shape, Point::new(200.0, 150.0)), Some(RED));
}

#[test]
fn pencil_straighten_makes_lines() {
    let mut e = editor();
    e.select_tool(ToolId::Pencil);
    e.state.settings.pencil_mode = anicraft_tools::PencilMode::Straighten;
    let events = vec![down(10.0, 10.0), mv(40.0, 11.0), mv(70.0, 9.0), mv(100.0, 10.0), mv(101.0, 40.0), mv(100.0, 70.0), up(100.0, 100.0)];
    let commands = send_all(&mut e, events);
    let (shape, _) = single_draw(&commands);
    assert!(shape.edges.iter().flat_map(|edge| edge.segments.iter()).all(|s| matches!(s, Segment::Line { .. })));
}

#[test]
fn brush_width_follows_pressure() {
    let mut e = editor();
    e.select_tool(ToolId::Brush);
    e.state.settings.brush_size = 20.0;
    let events: Vec<ToolEvent> = vec![
        PointerEvent::down(20.0, 100.0).with_pressure(0.2).into(),
        PointerEvent::drag(80.0, 100.0).with_pressure(0.5).into(),
        PointerEvent::drag(140.0, 100.0).with_pressure(1.0).into(),
        PointerEvent::up(200.0, 100.0).with_pressure(1.0).into(),
    ];
    let commands = send_all(&mut e, events);
    let (shape, _) = single_draw(&commands);
    assert_eq!(shape.strokes.len(), 0, "the brush paints fills");
    // Thin at the start, thick at the end.
    assert!(shape.fill_at(Point::new(30.0, 106.0)).is_none());
    assert!(shape.fill_at(Point::new(30.0, 100.0)).is_some());
    assert!(shape.fill_at(Point::new(190.0, 107.0)).is_some());
    assert!(shape.fill_at(Point::new(190.0, 112.0)).is_none());
    // Round cap.
    assert!(shape.fill_at(Point::new(205.0, 100.0)).is_some());
}

#[test]
fn brush_dab_is_a_disc_with_custom_colour() {
    let mut e = editor();
    e.select_tool(ToolId::Brush);
    e.state.settings.fill = FillStyle::Solid(Rgba8::opaque(0, 128, 0));
    e.state.settings.brush_size = 30.0;
    click(&mut e, ToolId::Brush, (100.0, 100.0));
    let shape = &shape_at(&e, 0).shape;
    assert_eq!(fill_color_at(shape, Point::new(100.0, 100.0)), Some(Rgba8::opaque(0, 128, 0)));
    assert!(shape.fill_at(Point::new(100.0, 120.0)).is_none());
}

#[test]
fn drawn_shapes_become_elements_on_the_right_layer_and_keyframe() {
    let mut e = editor();
    e.state.frame = 5;
    drag(&mut e, ToolId::Rectangle, (0.0, 0.0), (40.0, 40.0));
    // Frame 5 is inside the blank keyframe's span, so the shape lands in the keyframe at 0.
    let layer = &e.document.timeline(&e.state.timeline).unwrap().layers[0];
    assert!(layer.frames.iter().any(|f| f.elements.iter().any(|el| matches!(el, Element::Shape(_)))));
}
