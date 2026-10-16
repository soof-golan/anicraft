//! Selection, Subselection, Free Transform, Hand, Zoom and the tool set.

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

use anicraft_tools::{ElementRef, Key, KeyEvent, Modifiers, PointerEvent, Selection, ToolCommand, ToolEvent, ToolId, apply};
use anim_core::{Affine, Point, Rect, Vec2};
use anim_scene::{Element, TimelineId};
use common::*;

fn top(index: usize) -> ElementRef {
    ElementRef::top_level(0, 0, index)
}

fn two_rects() -> anicraft_tools::Editor {
    let mut e = editor();
    add_object_rect(&mut e, 50.0, 50.0, 150.0, 130.0, RED);
    add_object_rect(&mut e, 250.0, 60.0, 350.0, 160.0, BLUE);
    e
}

fn matrix_of(e: &anicraft_tools::Editor, index: usize) -> Affine {
    elements(e)[index].matrix().unwrap()
}

#[test]
fn click_selects_the_topmost_element_and_empty_space_clears() {
    let mut e = two_rects();
    let commands = click(&mut e, ToolId::Selection, (100.0, 90.0));
    assert_eq!(commands, vec![ToolCommand::SetSelection { selection: Selection::one(top(0)) }]);
    assert_eq!(e.state.selection, Selection::one(top(0)));
    click(&mut e, ToolId::Selection, (300.0, 100.0));
    assert_eq!(e.state.selection, Selection::one(top(1)));
    let commands = click(&mut e, ToolId::Selection, (450.0, 350.0));
    assert!(matches!(commands.as_slice(), [ToolCommand::SetSelection { selection }] if selection.is_empty()));
    assert!(e.state.selection.is_empty());
}

#[test]
fn overlapping_elements_pick_the_one_on_top() {
    let mut e = editor();
    add_object_rect(&mut e, 0.0, 0.0, 100.0, 100.0, RED);
    add_object_rect(&mut e, 50.0, 50.0, 150.0, 150.0, BLUE);
    click(&mut e, ToolId::Selection, (75.0, 75.0));
    assert_eq!(e.state.selection, Selection::one(top(1)));
    click(&mut e, ToolId::Selection, (25.0, 25.0));
    assert_eq!(e.state.selection, Selection::one(top(0)));
}

#[test]
fn shift_click_adds_and_removes() {
    let mut e = two_rects();
    click(&mut e, ToolId::Selection, (100.0, 90.0));
    send_all(
        &mut e,
        vec![PointerEvent::down(300.0, 100.0).with_modifiers(Modifiers::SHIFT).into(), PointerEvent::up(300.0, 100.0).with_modifiers(Modifiers::SHIFT).into()],
    );
    assert_eq!(e.state.selection.items, vec![top(0), top(1)]);
    send_all(
        &mut e,
        vec![PointerEvent::down(100.0, 90.0).with_modifiers(Modifiers::SHIFT).into(), PointerEvent::up(100.0, 90.0).with_modifiers(Modifiers::SHIFT).into()],
    );
    assert_eq!(e.state.selection.items, vec![top(1)]);
}

#[test]
fn marquee_selects_everything_it_touches() {
    let mut e = two_rects();
    e.select_tool(ToolId::Selection);
    let commands = send_all(&mut e, vec![down(20.0, 20.0), mv(200.0, 200.0), up(200.0, 200.0)]);
    assert_eq!(commands.len(), 1);
    assert_eq!(e.state.selection.items, vec![top(0)]);
    // A bigger marquee takes both; the overlay shows the rubber band while dragging.
    e.select_tool(ToolId::Selection);
    send_all(&mut e, vec![down(20.0, 20.0), mv(120.0, 120.0)]);
    let overlay = e.overlay();
    assert!(overlay.iter().any(|o| matches!(o, anicraft_tools::Overlay::Marquee { .. })));
    send_all(&mut e, vec![mv(400.0, 300.0), up(400.0, 300.0)]);
    assert_eq!(e.state.selection.items.len(), 2);
}

#[test]
fn shift_marquee_adds_to_the_selection() {
    let mut e = two_rects();
    click(&mut e, ToolId::Selection, (100.0, 90.0));
    let m = Modifiers::SHIFT;
    send_all(
        &mut e,
        vec![
            PointerEvent::down(240.0, 40.0).with_modifiers(m).into(),
            PointerEvent::drag(400.0, 200.0).with_modifiers(m).into(),
            PointerEvent::up(400.0, 200.0).with_modifiers(m).into(),
        ],
    );
    assert_eq!(e.state.selection.items.len(), 2);
}

#[test]
fn dragging_an_element_moves_it_with_one_transform_command() {
    let mut e = two_rects();
    e.select_tool(ToolId::Selection);
    let commands = send_all(&mut e, vec![down(100.0, 90.0), mv(120.0, 100.0), mv(160.0, 130.0), up(160.0, 130.0)]);
    let transforms: Vec<_> = commands.iter().filter(|c| matches!(c, ToolCommand::TransformSelection { .. })).collect();
    assert_eq!(transforms.len(), 1);
    assert_eq!(*transforms[0], ToolCommand::TransformSelection { matrix: Affine::translate(Vec2::new(60.0, 40.0)) });
    assert_eq!(matrix_of(&e, 0), Affine::translate(Vec2::new(60.0, 40.0)));
    assert_eq!(matrix_of(&e, 1), Affine::IDENTITY);
    assert_eq!(e.undo_depth(), 1);
    assert!(e.undo());
    assert_eq!(matrix_of(&e, 0), Affine::IDENTITY);
}

#[test]
fn moving_a_multi_selection_moves_all_and_shift_constrains() {
    let mut e = two_rects();
    e.select_tool(ToolId::Selection);
    send_all(&mut e, vec![down(20.0, 20.0), mv(400.0, 300.0), up(400.0, 300.0)]);
    assert_eq!(e.state.selection.items.len(), 2);
    let m = Modifiers::SHIFT;
    send_all(
        &mut e,
        vec![
            PointerEvent::down(100.0, 90.0).into(),
            PointerEvent::drag(160.0, 100.0).with_modifiers(m).into(),
            PointerEvent::up(160.0, 100.0).with_modifiers(m).into(),
        ],
    );
    for i in 0..2 {
        let t = matrix_of(&e, i).translation();
        // Shift keeps the length of the drag and snaps its direction to the horizontal.
        assert!((t.x - 60.0_f64.hypot(10.0)).abs() < 1.0e-9 && t.y.abs() < 1.0e-9, "element {i} moved horizontally: {t:?}");
    }
}

#[test]
fn click_without_drag_on_a_multi_selection_narrows_it() {
    let mut e = two_rects();
    e.select_tool(ToolId::Selection);
    send_all(&mut e, vec![down(20.0, 20.0), mv(400.0, 300.0), up(400.0, 300.0)]);
    let commands = send_all(&mut e, vec![down(100.0, 90.0), up(100.0, 90.0)]);
    assert!(commands.is_empty() || commands.iter().all(|c| matches!(c, ToolCommand::SetSelection { .. })));
    assert_eq!(e.state.selection.items, vec![top(0)]);
}

#[test]
fn alt_drag_duplicates_then_moves() {
    let mut e = two_rects();
    e.select_tool(ToolId::Selection);
    let a = Modifiers::ALT;
    let commands = send_all(
        &mut e,
        vec![
            PointerEvent::down(100.0, 90.0).with_modifiers(a).into(),
            PointerEvent::drag(120.0, 90.0).with_modifiers(a).into(),
            PointerEvent::up(200.0, 90.0).with_modifiers(a).into(),
        ],
    );
    assert!(commands.contains(&ToolCommand::DuplicateSelection));
    assert_eq!(elements(&e).len(), 3);
    assert_eq!(matrix_of(&e, 0), Affine::IDENTITY, "the original stays");
    assert_eq!(matrix_of(&e, 1).translation(), Vec2::new(100.0, 0.0), "the copy moved");
    assert_eq!(e.state.selection.items, vec![top(1)]);
}

#[test]
fn arrow_keys_nudge_and_delete_removes() {
    let mut e = two_rects();
    click(&mut e, ToolId::Selection, (100.0, 90.0));
    send_all(&mut e, vec![KeyEvent::press(Key::Right).into(), KeyEvent::press(Key::Down).into()]);
    send_all(&mut e, vec![KeyEvent::press(Key::Left).with_modifiers(Modifiers::SHIFT).into()]);
    assert_eq!(matrix_of(&e, 0).translation(), Vec2::new(-9.0, 1.0));
    send_all(&mut e, vec![KeyEvent::press(Key::Delete).into()]);
    assert_eq!(elements(&e).len(), 1);
    assert!(e.state.selection.is_empty());
}

#[test]
fn escape_clears_the_selection_and_cancels_drags() {
    let mut e = two_rects();
    click(&mut e, ToolId::Selection, (100.0, 90.0));
    send_all(&mut e, vec![down(100.0, 90.0), mv(200.0, 200.0), KeyEvent::press(Key::Escape).into(), up(200.0, 200.0)]);
    assert_eq!(matrix_of(&e, 0), Affine::IDENTITY);
    send_all(&mut e, vec![KeyEvent::press(Key::Escape).into()]);
    assert!(e.state.selection.is_empty());
}

#[test]
fn ctrl_a_selects_all_unlocked_elements() {
    let mut e = two_rects();
    e.select_tool(ToolId::Selection);
    send_all(&mut e, vec![KeyEvent::press(Key::Char('a')).with_modifiers(Modifiers::CTRL).into()]);
    assert_eq!(e.state.selection.items.len(), 2);
}

#[test]
fn locked_and_hidden_layers_cannot_be_picked() {
    let mut e = two_rects();
    e.document.timeline_mut(&TimelineId::Scene(0)).unwrap().layers[0].locked = true;
    click(&mut e, ToolId::Selection, (100.0, 90.0));
    assert!(e.state.selection.is_empty());
    e.document.timeline_mut(&TimelineId::Scene(0)).unwrap().layers[0].locked = false;
    e.document.timeline_mut(&TimelineId::Scene(0)).unwrap().layers[0].visible = false;
    click(&mut e, ToolId::Selection, (100.0, 90.0));
    assert!(e.state.selection.is_empty());
}

#[test]
fn double_click_enters_a_group_and_picks_its_members() {
    let mut e = editor();
    add_group(&mut e, vec![object_rect(10.0, 10.0, 60.0, 60.0, RED), object_rect(100.0, 10.0, 160.0, 60.0, BLUE)]);
    click(&mut e, ToolId::Selection, (30.0, 30.0));
    assert_eq!(e.state.selection, Selection::one(top(0)), "the group is one object");
    let commands = send_all(&mut e, vec![PointerEvent::down(30.0, 30.0).with_click_count(2).into(), up(30.0, 30.0)]);
    assert_eq!(commands, vec![ToolCommand::EnterGroup { target: top(0) }]);
    assert_eq!(e.state.selection.scope, Some(top(0)));
    // Inside the group, members are separate objects.
    click(&mut e, ToolId::Selection, (130.0, 30.0));
    assert_eq!(e.state.selection.items, vec![top(0).child(1)]);
    // Moving a member changes only that member.
    send_all(&mut e, vec![down(130.0, 30.0), mv(140.0, 30.0), up(150.0, 30.0)]);
    let Element::Group(g) = elements(&e)[0] else { panic!("group") };
    assert_eq!(g.members[0].matrix().unwrap(), Affine::IDENTITY);
    assert_eq!(g.members[1].matrix().unwrap().translation(), Vec2::new(20.0, 0.0));
    // Double-click on empty stage leaves the group.
    let commands = send_all(&mut e, vec![PointerEvent::down(400.0, 300.0).with_click_count(2).into(), up(400.0, 300.0)]);
    assert_eq!(commands, vec![ToolCommand::ExitScope]);
    assert_eq!(e.state.selection.scope, None);
}

#[test]
fn group_matrices_are_respected_when_moving_members() {
    let mut e = editor();
    let mut group = anim_scene::Group::new(vec![object_rect(0.0, 0.0, 50.0, 50.0, RED)]);
    group.matrix = Affine::scale(2.0) * Affine::translate(Vec2::new(10.0, 10.0));
    edit_add(&mut e, Element::Group(group));
    // The member covers stage x 20..120.
    click(&mut e, ToolId::Selection, (70.0, 70.0));
    send_all(&mut e, vec![PointerEvent::down(70.0, 70.0).with_click_count(2).into(), up(70.0, 70.0)]);
    click(&mut e, ToolId::Selection, (70.0, 70.0));
    send_all(&mut e, vec![down(70.0, 70.0), mv(100.0, 70.0), up(110.0, 70.0)]);
    let Element::Group(g) = elements(&e)[0] else { panic!("group") };
    // 40 stage pixels are 20 group pixels, i.e. 20 / 1 in the member's parent space (the group's own matrix scales).
    let t = g.members[0].matrix().unwrap().translation();
    assert!((t.x - 20.0).abs() < 1.0e-9 && t.y.abs() < 1.0e-9, "{t:?}");
}

fn edit_add(e: &mut anicraft_tools::Editor, element: Element) {
    anim_scene::edit::add_element(&mut e.document, &TimelineId::Scene(0), 0, 0, element).unwrap();
}

#[test]
fn double_click_enters_a_symbol_and_escape_leaves_it() {
    let mut e = editor();
    add_symbol_with_rect(&mut e, "box", (0.0, 0.0, 80.0, 60.0), (100.0, 100.0));
    click(&mut e, ToolId::Selection, (140.0, 130.0));
    assert_eq!(e.state.selection, Selection::one(top(0)), "the instance's bounds come from the symbol's content");
    let commands = send_all(&mut e, vec![PointerEvent::down(140.0, 130.0).with_click_count(2).into(), up(140.0, 130.0)]);
    assert_eq!(commands, vec![ToolCommand::EnterSymbol { target: top(0) }]);
    assert_eq!(e.state.timeline, TimelineId::Symbol("box".to_owned()));
    // Inside the symbol the rectangle is at its own coordinates.
    click(&mut e, ToolId::Selection, (40.0, 30.0));
    assert_eq!(e.state.selection.items.len(), 1);
    // Double-click on empty space inside a symbol returns to the scene.
    send_all(&mut e, vec![PointerEvent::down(400.0, 300.0).with_click_count(2).into(), up(400.0, 300.0)]);
    assert_eq!(e.state.timeline, TimelineId::Scene(0));
}

#[test]
fn rotated_symbol_instances_are_picked_by_their_rotated_bounds() {
    let mut e = editor();
    add_symbol_with_rect(&mut e, "bar", (0.0, 0.0, 200.0, 10.0), (100.0, 100.0));
    let id = TimelineId::Scene(0);
    let layer = &mut e.document.timeline_mut(&id).unwrap().layers[0];
    let Element::SymbolInstance(i) = &mut layer.frames[0].elements[0] else { panic!("instance") };
    i.matrix = Affine::translate(Vec2::new(100.0, 100.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
    // After a quarter turn the bar is vertical: it covers x 90..100, y 100..300.
    click(&mut e, ToolId::Selection, (95.0, 250.0));
    assert_eq!(e.state.selection.items.len(), 1);
    click(&mut e, ToolId::Selection, (200.0, 105.0));
    assert!(e.state.selection.is_empty(), "the unrotated position is empty now");
}

#[test]
fn dragging_an_edge_of_a_merge_shape_bends_it() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    e.select_tool(ToolId::Selection);
    let before = shape_at(&e, 0).shape.clone();
    // Grab the top edge in the middle and pull it up.
    let commands = send_all(&mut e, vec![down(200.0, 100.0), mv(200.0, 90.0), mv(200.0, 70.0), up(200.0, 70.0)]);
    assert!(commands.iter().any(|c| matches!(c, ToolCommand::ReplaceShape { .. })), "{commands:?}");
    let after = &shape_at(&e, 0).shape;
    assert_ne!(*after, before);
    assert!(after.fill_at(Point::new(200.0, 80.0)).is_some(), "the edge bulges upwards");
    assert!(after.fill_at(Point::new(110.0, 80.0)).is_none(), "the corners stay");
    assert!(after.fill_at(Point::new(200.0, 150.0)).is_some());
}

#[test]
fn hovering_reports_cursors() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    e.select_tool(ToolId::Selection);
    assert_eq!(e.send(&hover(200.0, 100.0)).unwrap().cursor, anicraft_tools::Cursor::Bend);
    assert_eq!(e.send(&hover(200.0, 150.0)).unwrap().cursor, anicraft_tools::Cursor::Move);
    assert_eq!(e.send(&hover(500.0, 350.0)).unwrap().cursor, anicraft_tools::Cursor::Default);
    // The hovered, unselected shape gets an overlay outline.
    let out = e.send(&hover(200.0, 150.0)).unwrap();
    assert!(out.overlay.iter().any(|o| matches!(o, anicraft_tools::Overlay::Rect { role: anicraft_tools::Role::Hover, .. })));
}

#[test]
fn subselection_moves_a_corner_point() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    click(&mut e, ToolId::Selection, (200.0, 150.0));
    e.select_tool(ToolId::Subselection);
    let commands = send_all(&mut e, vec![down(300.0, 200.0), mv(310.0, 220.0), up(330.0, 240.0)]);
    assert!(matches!(commands.as_slice(), [ToolCommand::ReplaceShape { .. }]), "{commands:?}");
    let shape = &shape_at(&e, 0).shape;
    let b = shape.bounds().unwrap();
    assert!((b.x1 - 330.0).abs() < 0.1 && (b.y1 - 240.0).abs() < 0.1, "{b:?}");
    assert!((b.x0 - 100.0).abs() < 0.1 && (b.y0 - 100.0).abs() < 0.1);
    assert!(shape.fill_at(Point::new(320.0, 235.0)).is_some());
    // The overlay shows square anchors.
    let overlay = e.overlay();
    let anchors = overlay.iter().filter(|o| matches!(o, anicraft_tools::Overlay::Marker { marker: anicraft_tools::Marker::Square, .. })).count();
    assert_eq!(anchors, 4);
}

#[test]
fn subselection_delete_removes_a_point_and_marquee_selects_points() {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    drag(&mut e, ToolId::Rectangle, (100.0, 100.0), (300.0, 200.0));
    click(&mut e, ToolId::Selection, (200.0, 150.0));
    e.select_tool(ToolId::Subselection);
    send_all(&mut e, vec![down(310.0, 90.0), mv(280.0, 120.0), up(290.0, 110.0)]);
    send_all(&mut e, vec![KeyEvent::press(Key::Delete).into()]);
    let shape = &shape_at(&e, 0).shape;
    let b = shape.bounds().unwrap();
    assert!((b.x1 - 300.0).abs() < 0.1);
    assert!(shape.fill_at(Point::new(290.0, 110.0)).is_none(), "the top right corner is gone");
    assert!(shape.fill_at(Point::new(150.0, 150.0)).is_some());
}

#[test]
fn free_transform_scales_about_the_opposite_corner() {
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 100.0, 200.0, 160.0, RED);
    click(&mut e, ToolId::Selection, (150.0, 130.0));
    e.select_tool(ToolId::FreeTransform);
    let commands = send_all(&mut e, vec![down(200.0, 160.0), mv(250.0, 190.0), up(300.0, 220.0)]);
    assert_eq!(commands.len(), 1);
    let ToolCommand::TransformSelection { matrix } = &commands[0] else { panic!("transform") };
    let c = matrix.as_coeffs();
    assert!((c[0] - 2.0).abs() < 1.0e-9 && (c[3] - 2.0).abs() < 1.0e-9, "{c:?}");
    #[allow(clippy::redundant_closure_for_method_calls, reason = "the method path does not satisfy the higher-ranked lifetime of with_context")]
    let b = e.with_context(|ctx| ctx.selection_bounds()).unwrap();
    assert!((b.x0 - 100.0).abs() < 1.0e-6 && (b.x1 - 300.0).abs() < 1.0e-6 && (b.y1 - 220.0).abs() < 1.0e-6, "{b:?}");
}

#[test]
fn free_transform_rotates_about_the_centre_with_snapping() {
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 100.0, 200.0, 200.0, RED);
    click(&mut e, ToolId::Selection, (150.0, 150.0));
    e.select_tool(ToolId::FreeTransform);
    // Outside the bottom-right corner is the rotate zone; swing from the corner to straight below the centre.
    let events: Vec<ToolEvent> = vec![
        PointerEvent::down(212.0, 212.0).into(),
        PointerEvent::drag(150.0, 230.0).with_modifiers(Modifiers::SHIFT).into(),
        PointerEvent::up(152.0, 230.0).with_modifiers(Modifiers::SHIFT).into(),
    ];
    let commands = send_all(&mut e, events);
    let ToolCommand::TransformSelection { matrix } = &commands[0] else { panic!("transform {commands:?}") };
    // The start direction was 45 degrees; the end is 90 degrees; shift snaps the difference (45) to 15-degree steps.
    let m = *matrix;
    let rotated = m * Point::new(200.0, 150.0);
    let angle = (rotated - Point::new(150.0, 150.0)).atan2();
    assert!((angle - std::f64::consts::FRAC_PI_4).abs() < 1.0e-6, "angle {angle}");
    assert!((m * Point::new(150.0, 150.0)).distance(Point::new(150.0, 150.0)) < 1.0e-9, "the centre stays");
}

#[test]
fn free_transform_skews_by_dragging_an_edge_and_moves_the_pivot() {
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 100.0, 200.0, 200.0, RED);
    click(&mut e, ToolId::Selection, (150.0, 150.0));
    e.select_tool(ToolId::FreeTransform);
    // The pointer is just above the top edge, between the handles.
    let commands = send_all(&mut e, vec![down(125.0, 96.0), mv(140.0, 96.0), up(145.0, 96.0)]);
    let ToolCommand::TransformSelection { matrix } = &commands[0] else { panic!("transform {commands:?}") };
    let top_left = *matrix * Point::new(100.0, 100.0);
    let bottom_left = *matrix * Point::new(100.0, 200.0);
    assert!((top_left.x - 120.0).abs() < 1.0e-9, "the top moves by the drag: {top_left:?}");
    assert!((bottom_left.x - 100.0).abs() < 1.0e-9, "the bottom stays: {bottom_left:?}");

    // Dragging the transformation point sets it on the element.
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 100.0, 200.0, 200.0, RED);
    click(&mut e, ToolId::Selection, (150.0, 150.0));
    e.select_tool(ToolId::FreeTransform);
    let commands = send_all(&mut e, vec![down(150.0, 150.0), mv(180.0, 160.0), up(190.0, 170.0)]);
    assert_eq!(commands, vec![ToolCommand::SetTransformationPoint { target: top(0), point: Point::new(190.0, 170.0) }]);
    let Element::Shape(s) = elements(&e)[0] else { panic!("shape") };
    assert_eq!(s.transformation_point, Some(Point::new(190.0, 170.0)));
}

#[test]
fn free_transform_with_alt_scales_about_the_transformation_point() {
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 100.0, 200.0, 200.0, RED);
    click(&mut e, ToolId::Selection, (150.0, 150.0));
    e.select_tool(ToolId::FreeTransform);
    let a = Modifiers::ALT;
    let commands = send_all(
        &mut e,
        vec![
            PointerEvent::down(200.0, 100.0).with_modifiers(a).into(),
            PointerEvent::drag(225.0, 75.0).with_modifiers(a).into(),
            PointerEvent::up(250.0, 50.0).with_modifiers(a).into(),
        ],
    );
    let ToolCommand::TransformSelection { matrix } = &commands[0] else { panic!("transform") };
    assert!((*matrix * Point::new(150.0, 150.0)).distance(Point::new(150.0, 150.0)) < 1.0e-9);
    assert!((*matrix * Point::new(200.0, 100.0)).distance(Point::new(250.0, 50.0)) < 1.0e-9);
}

#[test]
fn free_transform_moves_when_dragging_the_body() {
    let mut e = editor();
    add_object_rect(&mut e, 100.0, 100.0, 200.0, 200.0, RED);
    click(&mut e, ToolId::Selection, (120.0, 120.0));
    e.select_tool(ToolId::FreeTransform);
    let commands = send_all(&mut e, vec![down(130.0, 130.0), mv(140.0, 140.0), up(150.0, 130.0)]);
    assert_eq!(commands, vec![ToolCommand::TransformSelection { matrix: Affine::translate(Vec2::new(20.0, 0.0)) }]);
}

#[test]
fn hand_pans_in_screen_space_and_zoom_zooms() {
    let mut e = editor();
    e.select_tool(ToolId::Hand);
    let events: Vec<ToolEvent> = vec![
        PointerEvent::down(100.0, 100.0).with_screen(300.0, 300.0).into(),
        PointerEvent::drag(100.0, 100.0).with_screen(320.0, 310.0).into(),
        PointerEvent::drag(100.0, 100.0).with_screen(330.0, 305.0).into(),
        PointerEvent::up(100.0, 100.0).with_screen(330.0, 305.0).into(),
    ];
    let commands = send_all(&mut e, events);
    assert_eq!(commands, vec![ToolCommand::PanScreen { delta: Vec2::new(20.0, 10.0) }, ToolCommand::PanScreen { delta: Vec2::new(10.0, -5.0) }]);
    assert_eq!(e.state.view.offset, Vec2::new(30.0, 5.0));

    let commands = click(&mut e, ToolId::Zoom, (100.0, 100.0));
    assert_eq!(commands, vec![ToolCommand::ZoomAt { center: Point::new(100.0, 100.0), factor: 2.0 }]);
    assert!((e.state.view.zoom - 2.0).abs() < 1.0e-12);
    e.select_tool(ToolId::Zoom);
    let alt = Modifiers::ALT;
    let commands = send_all(&mut e, vec![PointerEvent::down(50.0, 50.0).with_modifiers(alt).into(), PointerEvent::up(50.0, 50.0).with_modifiers(alt).into()]);
    assert_eq!(commands, vec![ToolCommand::ZoomAt { center: Point::new(50.0, 50.0), factor: 0.5 }]);
    let commands = drag(&mut e, ToolId::Zoom, (0.0, 0.0), (200.0, 100.0));
    assert_eq!(commands, vec![ToolCommand::ZoomToRect { rect: Rect::new(0.0, 0.0, 200.0, 100.0) }]);
    assert!((e.state.view.zoom - 5.0).abs() < 1.0e-9);
}

#[test]
fn space_bar_pans_temporarily_and_letters_switch_tools() {
    let mut e = editor();
    e.select_tool(ToolId::Rectangle);
    send_all(&mut e, vec![KeyEvent::press(Key::Space).into()]);
    assert_eq!(e.tools.active(), ToolId::Hand);
    send_all(
        &mut e,
        vec![
            PointerEvent::down(0.0, 0.0).with_screen(10.0, 10.0).into(),
            PointerEvent::drag(0.0, 0.0).with_screen(15.0, 10.0).into(),
            PointerEvent::up(0.0, 0.0).with_screen(15.0, 10.0).into(),
        ],
    );
    assert_eq!(e.state.view.offset, Vec2::new(5.0, 0.0));
    send_all(&mut e, vec![KeyEvent::release(Key::Space).into()]);
    assert_eq!(e.tools.active(), ToolId::Rectangle);
    assert!(elements(&e).is_empty(), "nothing was drawn while panning");

    for tool in ToolId::ALL.into_iter().filter(|t| t.shortcut() != '\0') {
        let c = send_all(&mut e, vec![KeyEvent::press(Key::Char(tool.shortcut())).into()]);
        assert_eq!(c, vec![ToolCommand::ActivateTool { tool }]);
        assert_eq!(e.tools.active(), tool);
        assert_eq!(e.state.active_tool, tool);
    }
}

#[test]
fn apply_reports_errors_for_stale_references() {
    let mut e = two_rects();
    let bogus = ToolCommand::ReplaceShape { target: ElementRef::top_level(0, 0, 9), shape: rect_shape(0.0, 0.0, 1.0, 1.0, Some(RED), None) };
    assert!(apply(&mut e.document, &mut e.state, &bogus).is_err());
    let text_on_shape = ToolCommand::InsertText { target: top(0), offset: 0, text: "x".to_owned() };
    assert!(apply(&mut e.document, &mut e.state, &text_on_shape).is_err());
    assert_eq!(elements(&e).len(), 2);
}
