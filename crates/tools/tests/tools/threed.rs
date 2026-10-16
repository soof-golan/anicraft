//! The 3D Rotation and 3D Translation tools and the 3D MCP commands, with the corner positions computed by hand.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::float_cmp)]

use crate::common::{add_symbol_with_rect, click, drag, editor};

use anicraft_tools::{Editor, ToolCommand, ToolId};
use anim_core::{Matrix3D, Point};
use anim_scene::{Element, SymbolInstance, TimelineId};

fn instance(e: &Editor) -> &SymbolInstance {
    let layer = &e.document.timeline(&TimelineId::Scene(0)).unwrap().layers[0];
    let Element::SymbolInstance(i) = &layer.frames[0].elements[0] else { panic!("instance") };
    i
}

/// A 100 x 100 box whose origin is at (200, 150), selected.
fn boxed() -> Editor {
    let mut e = editor();
    add_symbol_with_rect(&mut e, "box", (0.0, 0.0, 100.0, 100.0), (200.0, 150.0));
    click(&mut e, ToolId::Selection, (250.0, 200.0));
    assert_eq!(e.state.selection.items.len(), 1);
    e
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1.0e-6)
}

#[test]
fn translating_along_z_gives_the_instance_depth_and_leaves_x_and_y() {
    let mut e = boxed();
    // The z handle is the diagonal up and to the right of the centre (200, 150); 10 along it is (207.07, 142.93).
    let d = 10.0 / 2.0_f64.sqrt();
    let from = (200.0 + d, 150.0 - d);
    let to = (3.0_f64.mul_add(d, 200.0), 3.0_f64.mul_add(-d, 150.0));
    let commands = drag(&mut e, ToolId::Translation3D, from, to);
    assert!(matches!(commands.last(), Some(ToolCommand::Transform3DSelection { .. })), "{commands:?}");
    let i = instance(&e);
    let m = i.matrix_3d.expect("a 3D placement");
    // The pointer moved 20 along the handle's direction.
    assert!(near(m.transform_point([0.0, 0.0, 0.0]), [200.0, 150.0, 20.0]), "{:?}", m.transform_point([0.0, 0.0, 0.0]));
    // The 2D matrix keeps the flattened plane.
    assert_eq!((i.matrix.translation().x, i.matrix.translation().y), (200.0, 150.0));
}

#[test]
fn translating_along_x_in_the_plane_stays_a_plain_2d_instance() {
    let mut e = boxed();
    drag(&mut e, ToolId::Translation3D, (230.0, 150.0), (255.0, 190.0));
    let i = instance(&e);
    assert!(i.matrix_3d.is_none());
    assert_eq!((i.matrix.translation().x, i.matrix.translation().y), (225.0, 150.0));
}

#[test]
fn rotating_about_y_turns_the_box_about_its_origin() {
    let mut e = boxed();
    // The y line runs vertically through the centre; dragging left by 45 points turns 45 degrees, right edge forward.
    drag(&mut e, ToolId::Rotation3D, (200.0, 100.0), (155.0, 100.0));
    let m = instance(&e).matrix_3d.expect("a 3D placement");
    let s = 45.0_f64.to_radians();
    assert!(near(m.transform_point([0.0, 0.0, 0.0]), [200.0, 150.0, 0.0]));
    assert!(
        near(m.transform_point([100.0, 0.0, 0.0]), [100.0_f64.mul_add(s.cos(), 200.0), 150.0, -100.0 * s.sin()]),
        "{:?}",
        m.transform_point([100.0, 0.0, 0.0])
    );
    assert!(near(m.transform_point([0.0, 100.0, 0.0]), [200.0, 250.0, 0.0]));
}

#[test]
fn rotating_about_the_centre_point_keeps_it_fixed() {
    let mut e = boxed();
    // Put the centre point at the middle of the box and turn about x.
    let target = e.state.selection.items[0].clone();
    e.run_commands(&[ToolCommand::SetCenterPoint3D { target, point: Some(Point::new(250.0, 200.0)) }]).unwrap();
    // The x line runs horizontally through (250, 200); drag down by 90 points turns 90 degrees about x.
    drag(&mut e, ToolId::Rotation3D, (280.0, 200.0), (280.0, 290.0));
    let m = instance(&e).matrix_3d.expect("a 3D placement");
    // Local (50, 50) is the centre of the box and stays; local (50, 0) is 50 above it and goes to depth.
    assert!(near(m.transform_point([50.0, 50.0, 0.0]), [250.0, 200.0, 0.0]), "{:?}", m.transform_point([50.0, 50.0, 0.0]));
    let top = m.transform_point([50.0, 0.0, 0.0]);
    assert!((top[0] - 250.0).abs() < 1.0e-6 && (top[1] - 200.0).abs() < 1.0e-6 && (top[2].abs() - 50.0).abs() < 1.0e-6, "{top:?}");
}

#[test]
fn the_3d_tools_ignore_selections_without_symbol_instances() {
    let mut e = editor();
    let commands = drag(&mut e, ToolId::Rotation3D, (200.0, 100.0), (155.0, 100.0));
    assert_eq!(commands.len(), 0);
}

#[test]
fn the_transform_is_undone_by_snapshot_and_survives_a_matrix_roundtrip() {
    let mut e = boxed();
    let before = e.document.clone();
    e.run_commands(&[ToolCommand::Transform3DSelection { matrix: Matrix3D::rotation_y(30.0) }]).unwrap();
    assert!(instance(&e).matrix_3d.is_some());
    assert_ne!(e.document, before);
    e.undo();
    assert_eq!(e.document, before);
}
