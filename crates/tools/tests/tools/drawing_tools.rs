//! Primitives, `PolyStar`, the Lasso and the anchor point tools.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::float_cmp,
    clippy::manual_assert_eq,
    clippy::suboptimal_flops,
    clippy::many_single_char_names
)]

use crate::common;

use anicraft_tools::settings::{LassoMode, PolyStarStyle};
use anicraft_tools::{ElementRef, Modifiers, PointerEvent, Selection, ToolCommand, ToolEvent, ToolId};
use anim_core::Point;
use anim_scene::{Element, Primitive, PrimitiveKind};
use common::*;

fn select(e: &mut anicraft_tools::Editor, index: usize) {
    e.state.selection = Selection { items: vec![ElementRef::top_level(0, 0, index)], scope: None };
}

fn primitive_of(e: &anicraft_tools::Editor, index: usize) -> Primitive {
    *shape_at(e, index).primitive.clone().expect("a primitive")
}

#[test]
fn the_rectangle_primitive_stays_editable() {
    let mut e = editor();
    e.state.settings.corner_lock = false;
    e.state.settings.corner_radii = [20.0, 0.0, 0.0, 0.0];
    drag(&mut e, ToolId::RectanglePrimitive, (50.0, 40.0), (250.0, 140.0));
    assert_eq!(elements(&e).len(), 1);
    let element = shape_at(&e, 0);
    assert!(element.is_drawing_object);
    let PrimitiveKind::Rectangle(r) = &primitive_of(&e, 0).kind else { panic!("not a rectangle") };
    assert_eq!((r.width, r.height), (200.0, 100.0));
    assert_eq!(r.radii, [20.0, 0.0, 0.0, 0.0]);
    // Placed by its matrix, geometry at the origin of its own space.
    assert_eq!(element.matrix.translation(), (50.0, 40.0).into());
    assert!(element.shape.fill_at(Point::new(2.0, 2.0)).is_none(), "the rounded corner is cut away");
    assert!(element.shape.fill_at(Point::new(198.0, 2.0)).is_some());

    // Change the radii and the size through the command.
    let mut primitive = primitive_of(&e, 0);
    if let PrimitiveKind::Rectangle(r) = &mut primitive.kind {
        r.radii = [0.0, 0.0, 0.0, 30.0];
        r.width = 120.0;
    }
    select(&mut e, 0);
    e.run_commands(&[ToolCommand::SetPrimitive { target: ElementRef::top_level(0, 0, 0), primitive }]).unwrap();
    let element = shape_at(&e, 0);
    assert!(element.shape.fill_at(Point::new(2.0, 2.0)).is_some(), "the top left corner is square again");
    assert!(element.shape.fill_at(Point::new(2.0, 98.0)).is_none(), "the bottom left corner is rounded now");
    assert!(element.shape.fill_at(Point::new(150.0, 50.0)).is_none(), "the width shrank to 120");
    assert!(e.undo());
    let PrimitiveKind::Rectangle(r) = &primitive_of(&e, 0).kind else { panic!("not a rectangle") };
    assert_eq!(r.width, 200.0);
}

#[test]
fn the_oval_primitive_keeps_its_arc() {
    let mut e = editor();
    e.state.settings.oval = anicraft_tools::OvalSettings { start_angle: 0.0, end_angle: 90.0, inner_radius: 0.0, close_path: true };
    drag(&mut e, ToolId::OvalPrimitive, (100.0, 100.0), (300.0, 300.0));
    let element = shape_at(&e, 0);
    let PrimitiveKind::Oval(o) = &primitive_of(&e, 0).kind else { panic!("not an oval") };
    assert_eq!((o.start_angle, o.end_angle), (0.0, 90.0));
    assert!(element.shape.fill_at(Point::new(150.0, 150.0)).is_some(), "the quarter between three and six o'clock, in the oval's space");
    assert!(element.shape.fill_at(Point::new(50.0, 50.0)).is_none());
}

#[test]
fn editing_the_outline_by_hand_ends_the_primitive() {
    let mut e = editor();
    drag(&mut e, ToolId::RectanglePrimitive, (50.0, 40.0), (250.0, 140.0));
    let shape = shape_at(&e, 0).shape.clone();
    e.run_commands(&[ToolCommand::ReplaceShape { target: ElementRef::top_level(0, 0, 0), shape }]).unwrap();
    assert!(shape_at(&e, 0).primitive.is_none());
    let primitive = Primitive {
        kind: PrimitiveKind::Oval(anim_scene::OvalPrimitive { width: 1.0, height: 1.0, start_angle: 0.0, end_angle: 0.0, inner_radius: 0.0, close_path: true }),
        fill: None,
        stroke: None,
    };
    assert!(
        e.run_commands(&[ToolCommand::SetPrimitive { target: ElementRef::top_level(0, 0, 0), primitive }]).is_err(),
        "a plain shape has no primitive to set"
    );
}

#[test]
fn polystar_draws_polygons_and_stars() {
    let mut e = editor();
    e.state.settings.polystar.sides = 5;
    e.state.settings.polystar.style = PolyStarStyle::Polygon;
    drag(&mut e, ToolId::PolyStar, (200.0, 200.0), (300.0, 200.0));
    let polygon = &shape_at(&e, 0).shape;
    assert!(polygon.fill_at(Point::new(200.0, 200.0)).is_some(), "the centre is filled");
    let b = polygon.bounds().unwrap();
    assert!((b.x1 - 300.0).abs() < 0.1, "the first corner is where the drag ended: {b:?}");

    let mut e = editor();
    e.state.settings.polystar.sides = 5;
    e.state.settings.polystar.style = PolyStarStyle::Star;
    e.state.settings.polystar.star_point_size = 0.3;
    drag(&mut e, ToolId::PolyStar, (200.0, 200.0), (300.0, 200.0));
    let star = &shape_at(&e, 0).shape;
    // Between two points of the star, at the outer radius, there is nothing.
    let gap = std::f64::consts::TAU / 10.0;
    let outside = Point::new(200.0 + 90.0 * gap.cos(), 200.0 + 90.0 * gap.sin());
    assert!(star.fill_at(outside).is_none(), "the star has notches");
    assert!(star.fill_at(Point::new(200.0, 200.0)).is_some());
}

#[test]
fn a_freeform_lasso_selects_the_part_of_a_shape_inside_it() {
    let mut e = editor();
    let shape = rect_shape(100.0, 100.0, 300.0, 200.0, Some(RED), None);
    anim_scene::edit::add_element(&mut e.document, &e.state.timeline.clone(), 0, 0, Element::Shape(anim_scene::ShapeElement::new(shape))).unwrap();
    e.select_tool(ToolId::Lasso);
    // A loop around the left half of the rectangle.
    let events = vec![down(50.0, 50.0), mv(200.0, 50.0), mv(200.0, 250.0), mv(50.0, 250.0), up(50.0, 250.0)];
    send_all(&mut e, events);
    assert_eq!(elements(&e).len(), 2, "the shape was cut along the loop");
    assert_eq!(e.state.selection.items.len(), 1);
    let Element::Shape(part) = &elements(&e)[e.state.selection.items[0].path[0]] else { panic!("not a shape") };
    let b = part.shape.bounds().unwrap();
    assert!((b.x0 - 100.0).abs() < 0.1 && (b.x1 - 200.0).abs() < 0.1, "the selected piece is the left half: {b:?}");
}

#[test]
fn the_polygon_lasso_collects_corners_and_closes_on_enter() {
    let mut e = editor();
    let shape = rect_shape(100.0, 100.0, 300.0, 200.0, Some(RED), None);
    anim_scene::edit::add_element(&mut e.document, &e.state.timeline.clone(), 0, 0, Element::Shape(anim_scene::ShapeElement::new(shape))).unwrap();
    e.state.settings.lasso.mode = LassoMode::Polygon;
    e.select_tool(ToolId::Lasso);
    for (x, y) in [(50.0, 50.0), (200.0, 50.0), (200.0, 250.0), (50.0, 250.0)] {
        send_all(&mut e, vec![down(x, y), up(x, y)]);
    }
    assert!(e.state.selection.items.is_empty(), "nothing is selected before the polygon closes");
    e.send(&ToolEvent::Key(anicraft_tools::KeyEvent { key: anicraft_tools::Key::Enter, modifiers: Modifiers::NONE, pressed: true })).unwrap();
    assert_eq!(e.state.selection.items.len(), 1);
    assert_eq!(elements(&e).len(), 2);
}

#[test]
fn the_magic_wand_selects_fills_of_a_similar_colour() {
    let mut e = editor();
    let timeline = e.state.timeline.clone();
    for (x0, colour) in [(0.0, RED), (100.0, BLUE), (200.0, anim_core::Rgba8::opaque(250, 4, 0))] {
        let shape = rect_shape(x0, 0.0, x0 + 50.0, 50.0, Some(colour), None);
        anim_scene::edit::add_element(&mut e.document, &timeline, 0, 0, Element::Shape(anim_scene::ShapeElement::new(shape))).unwrap();
    }
    e.state.settings.lasso.mode = LassoMode::MagicWand;
    e.state.settings.lasso.threshold = 20.0;
    click(&mut e, ToolId::Lasso, (10.0, 10.0));
    assert_eq!(e.state.selection.items.len(), 2, "the red and the nearly red shape, not the blue one");
}

fn square_element(e: &mut anicraft_tools::Editor) {
    let shape = rect_shape(100.0, 100.0, 200.0, 200.0, Some(RED), None);
    anim_scene::edit::add_element(&mut e.document, &e.state.timeline.clone(), 0, 0, Element::Shape(anim_scene::ShapeElement::new(shape))).unwrap();
}

fn segment_count(e: &anicraft_tools::Editor) -> usize {
    shape_at(e, 0).shape.edges.iter().map(|edge| edge.segments.len()).sum()
}

#[test]
fn the_anchor_tools_add_delete_and_convert() {
    let mut e = editor();
    square_element(&mut e);
    let before = segment_count(&e);
    click(&mut e, ToolId::AddAnchor, (150.0, 100.0));
    assert_eq!(segment_count(&e), before + 1, "an anchor was added on the top edge");
    assert!(shape_at(&e, 0).shape.fill_at(Point::new(150.0, 150.0)).is_some());

    click(&mut e, ToolId::DeleteAnchor, (150.0, 100.0));
    assert_eq!(segment_count(&e), before, "the anchor is gone again");

    click(&mut e, ToolId::ConvertAnchor, (200.0, 100.0));
    let smooth = shape_at(&e, 0).shape.edges.iter().flat_map(|edge| edge.segments.iter()).any(|s| matches!(s, anim_shape::Segment::Cubic { .. }));
    assert!(smooth, "the corner became smooth");
    click(&mut e, ToolId::ConvertAnchor, (200.0, 100.0));
    assert!(e.undo());
}

/// True when every segment of a stroke edge has a twin, on the same curve, in an edge that carries a fill.
fn strokes_follow_the_fills(e: &anicraft_tools::Editor) -> bool {
    let shape = &shape_at(e, 0).shape;
    let fills: Vec<anim_shape::Segment> = shape.edges.iter().filter(|x| x.fill_style0 != 0 || x.fill_style1 != 0).flat_map(|x| x.segments.clone()).collect();
    shape.edges.iter().filter(|x| x.stroke_style != 0).flat_map(|x| x.segments.iter()).all(|s| fills.iter().any(|f| f == s || *f == s.reversed()))
}

#[test]
fn anchor_edits_keep_the_fill_and_the_stroke_on_the_same_curve() {
    let mut e = editor();
    e.state.settings.object_drawing = false;
    drag(&mut e, ToolId::Rectangle, (40.0, 40.0), (140.0, 120.0));
    drag(&mut e, ToolId::Rectangle, (240.0, 40.0), (340.0, 120.0));
    assert_eq!(elements(&e).len(), 1, "both rectangles merged into one shape");
    select(&mut e, 0);
    assert!(strokes_follow_the_fills(&e), "a freshly drawn outline has fill and stroke edges on the same curves");
    let before = segment_count(&e);
    click(&mut e, ToolId::AddAnchor, (290.0, 40.0));
    assert!(segment_count(&e) > before, "an anchor was added");
    assert!(strokes_follow_the_fills(&e), "the stroke edge was split where the fill edge was");
    drag(&mut e, ToolId::ConvertAnchor, (340.0, 40.0), (360.0, 20.0));
    let cubic = |edge: &anim_shape::Edge| edge.segments.iter().any(|s| matches!(s, anim_shape::Segment::Cubic { .. }));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.edges.iter().filter(|x| x.stroke_style != 0).any(cubic), "the stroke curves too");
    assert!(shape.edges.iter().filter(|x| x.fill_style1 != 0).any(cubic), "the fill curves");
    assert!(strokes_follow_the_fills(&e), "the stroke edge follows the fill edge through the conversion");
    let again = segment_count(&e);
    click(&mut e, ToolId::DeleteAnchor, (290.0, 40.0));
    assert!(segment_count(&e) < again, "deleting the added anchor joins its neighbours");
    assert!(strokes_follow_the_fills(&e));
}

#[test]
fn the_pen_modifies_a_selected_path_instead_of_starting_a_new_one() {
    let mut e = editor();
    square_element(&mut e);
    select(&mut e, 0);
    let before = segment_count(&e);
    click(&mut e, ToolId::Pen, (150.0, 100.0));
    assert_eq!(segment_count(&e), before + 1, "a click on a segment adds an anchor");
    click(&mut e, ToolId::Pen, (150.0, 100.0));
    assert_eq!(segment_count(&e), before, "a click on an anchor deletes it");
    let alt = Modifiers::ALT;
    e.select_tool(ToolId::Pen);
    send_all(&mut e, vec![PointerEvent::down(200.0, 100.0).with_modifiers(alt).into(), PointerEvent::up(200.0, 100.0).with_modifiers(alt).into()]);
    assert!(
        shape_at(&e, 0).shape.edges.iter().flat_map(|edge| edge.segments.iter()).any(|s| matches!(s, anim_shape::Segment::Cubic { .. })),
        "alt-clicking an anchor converts it"
    );
}

fn pen_preview_segments(e: &anicraft_tools::Editor) -> usize {
    e.overlay()
        .iter()
        .filter_map(|o| match o {
            anicraft_tools::Overlay::Path { path, role: anicraft_tools::Role::Preview, .. } => Some(path.elements().len()),
            _ => None,
        })
        .sum()
}

#[test]
fn the_pen_preview_preference_hides_the_rubber_band() {
    let mut e = editor();
    e.select_tool(ToolId::Pen);
    send_all(&mut e, vec![down(50.0, 50.0), up(50.0, 50.0), hover(120.0, 90.0)]);
    let with_band = pen_preview_segments(&e);
    e.state.settings.pen_preview = false;
    let without_band = pen_preview_segments(&e);
    assert!(with_band > without_band, "the rubber band adds a segment from the last anchor to the pointer");
}

#[test]
fn the_subselection_tool_converts_an_anchor_with_alt() {
    let mut e = editor();
    square_element(&mut e);
    select(&mut e, 0);
    e.select_tool(ToolId::Subselection);
    let alt = Modifiers::ALT;
    send_all(&mut e, vec![PointerEvent::down(200.0, 100.0).with_modifiers(alt).into(), PointerEvent::up(200.0, 100.0).with_modifiers(alt).into()]);
    assert!(shape_at(&e, 0).shape.edges.iter().flat_map(|edge| edge.segments.iter()).any(|s| matches!(s, anim_shape::Segment::Cubic { .. })));
}

fn thick_line(e: &mut anicraft_tools::Editor) {
    e.state.settings.stroke = anim_shape::StrokeStyle::plain(4.0, anim_core::Rgba8::BLACK);
    drag(e, ToolId::Line, (50.0, 100.0), (250.0, 100.0));
}

#[test]
fn the_width_tool_widens_a_stroke_where_it_is_dragged() {
    let mut e = editor();
    thick_line(&mut e);
    assert_eq!(shape_at(&e, 0).shape.strokes.len(), 1);
    drag(&mut e, ToolId::Width, (150.0, 100.0), (150.0, 120.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.strokes.is_empty(), "the stroke became a fill");
    assert!(shape.fill_at(Point::new(150.0, 115.0)).is_some(), "the line is 40 px wide where it was dragged");
    assert!(shape.fill_at(Point::new(150.0, 85.0)).is_some(), "on both sides");
    assert!(shape.fill_at(Point::new(60.0, 110.0)).is_none(), "far from the point it is still thin");
    assert!(shape.fill_at(Point::new(60.0, 101.0)).is_some());

    // A second drag on the same shape refines the same profile.
    drag(&mut e, ToolId::Width, (100.0, 100.0), (100.0, 130.0));
    let shape = &shape_at(&e, 0).shape;
    assert!(shape.fill_at(Point::new(100.0, 125.0)).is_some(), "the second point widened the line at x = 100");
    assert!(shape.fill_at(Point::new(150.0, 115.0)).is_some(), "the first point is still there");
    assert!(e.undo());
    assert!(shape_at(&e, 0).shape.fill_at(Point::new(100.0, 125.0)).is_none(), "undo takes the last width point back");
}

#[test]
fn width_presets_taper_the_ends() {
    let mut e = editor();
    thick_line(&mut e);
    e.state.settings.stroke = anim_shape::StrokeStyle::plain(12.0, anim_core::Rgba8::BLACK);
    drag(&mut e, ToolId::Line, (50.0, 200.0), (250.0, 200.0));
    let shape = shape_at(&e, 0).shape.clone();
    let baked = anicraft_tools::widthline::apply_profile(&shape, &anicraft_tools::WidthProfile::TaperBoth).unwrap();
    assert!(baked.fill_at(Point::new(150.0, 205.0)).is_some());
    assert!(baked.fill_at(Point::new(52.0, 205.0)).is_none(), "pointed end");
}

#[test]
fn art_and_pattern_brushes_paint_filled_shapes() {
    use anicraft_tools::PaintBrushStyle;
    for style in PaintBrushStyle::ALL {
        let mut e = editor();
        e.state.settings.paint_brush.style = style;
        e.state.settings.paint_brush.size = 14.0;
        drag(&mut e, ToolId::PaintBrush, (60.0, 100.0), (260.0, 130.0));
        assert_eq!(elements(&e).len(), 1, "{style:?}");
        let shape = &shape_at(&e, 0).shape;
        assert!(shape.strokes.is_empty() && !shape.fills.is_empty(), "{style:?} paints fills");
        let b = shape.bounds().unwrap();
        assert!(b.x0 < 80.0 && b.x1 > 240.0, "{style:?}: {b:?}");
    }
    // A pattern brush leaves gaps between its tiles.
    let mut e = editor();
    e.state.settings.paint_brush.style = PaintBrushStyle::Dots;
    e.state.settings.paint_brush.size = 8.0;
    e.state.settings.paint_brush.spacing = 200.0;
    drag(&mut e, ToolId::PaintBrush, (60.0, 100.0), (260.0, 100.0));
    let shape = &shape_at(&e, 0).shape;
    let hits = (0..200).filter(|i| shape.fill_at(Point::new(60.0 + f64::from(*i), 100.0)).is_some()).count();
    assert!(hits > 20 && hits < 120, "dots with gaps cover part of the line: {hits}");
}
