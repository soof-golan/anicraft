//! The 3D Rotation and 3D Translation tools: turn and move the selected symbol instances in 3D.
//!
//! Both draw a gizmo at the instance's centre point (`centerPoint3D`, or the instance's own transformation point):
//!
//! * **3D Translation**: the x axis (a line to the right) moves along x, the y axis (a line down) along y, the z axis
//!   (a diagonal up and to the right) along z, away from the viewer.
//! * **3D Rotation**: the horizontal line turns the object about the x axis (drag up or down), the vertical line
//!   about the y axis (drag left or right), the circle about the z axis, and the ring outside it about x and y at
//!   once. Dragging turns the surface as if it were a ball: dragging right turns the front to the right.
//!
//! Rotations and translations are global (about the stage axes). Shift snaps rotations to 15 degrees. The gesture
//! shows the projected outline of each instance while the pointer is down and emits one
//! [`ToolCommand::Transform3DSelection`] on release.

use anim_core::kurbo::{Circle, Shape};
use anim_core::{Affine, Matrix3D, Point};
use anim_scene::{Element, SymbolInstance};

use crate::command::ToolCommand;
use crate::context::{Resolved, ToolContext};
use crate::event::{Key, Modifiers, PointerKind, ToolEvent};
use crate::overlay::{Cursor, Marker, Overlay, Role};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Length of the gizmo axes and radius of its circle, in screen points.
const SIZE: f64 = 70.0;
/// The outer ring of the rotation gizmo reaches this many times the radius.
const OUTER: f64 = 1.3;
/// Degrees of rotation per screen point of drag.
const DEGREES_PER_POINT: f64 = 1.0;
/// Rotation snap with shift, degrees.
const SNAP: f64 = 15.0;

/// Which part of the gizmo the pointer grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grip {
    X,
    Y,
    Z,
    /// The outer ring of the rotation gizmo: x and y at once.
    Free,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Rotate,
    Translate,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    grip: Grip,
    start: Point,
    current: Point,
    modifiers: Modifiers,
    /// The gizmo centre when the gesture began: stage x and y, and the depth.
    centre: [f64; 3],
}

#[derive(Debug, Clone, Copy)]
struct Gizmo {
    mode: Mode,
    drag: Option<Drag>,
}

/// The selected symbol instances with their stage transforms.
fn instances<'a>(ctx: &ToolContext<'a>) -> Vec<Resolved<'a>> {
    ctx.selection.items.iter().filter_map(|r| ctx.resolve(r)).filter(|r| matches!(r.element, Element::SymbolInstance(_))).collect()
}

fn instance_of<'a>(resolved: &Resolved<'a>) -> Option<&'a SymbolInstance> {
    match resolved.element {
        Element::SymbolInstance(i) => Some(i),
        _ => None,
    }
}

/// The centre of one instance: its centre point if set, else where its transformation point is now. Stage x and y,
/// and the depth.
fn centre_of(resolved: &Resolved<'_>) -> Option<[f64; 3]> {
    let instance = instance_of(resolved)?;
    let origin = instance.placement_3d().transform_point([instance.transformation_point.x, instance.transformation_point.y, 0.0]);
    let [x, y, z] = instance.center_point_3d.map_or(origin, |p| [p.x, p.y, origin[2]]);
    let stage = resolved.parent_to_stage * Point::new(x, y);
    (stage.x.is_finite() && stage.y.is_finite() && z.is_finite()).then_some([stage.x, stage.y, z])
}

/// The mean of the instances' centres.
pub fn gizmo_centre(ctx: &ToolContext<'_>) -> Option<[f64; 3]> {
    let centres: Vec<[f64; 3]> = instances(ctx).iter().filter_map(centre_of).collect();
    let count = u32::try_from(centres.len()).ok().filter(|n| *n > 0)?;
    let count = f64::from(count);
    let sum = centres.iter().fold([0.0; 3], |acc, c| [acc[0] + c[0], acc[1] + c[1], acc[2] + c[2]]);
    Some([sum[0] / count, sum[1] / count, sum[2] / count])
}

const fn at(centre: [f64; 3]) -> Point {
    Point::new(centre[0], centre[1])
}

/// Direction of the z axis handle on screen (up and to the right), a unit vector.
const Z_DIRECTION: (f64, f64) = (std::f64::consts::FRAC_1_SQRT_2, -std::f64::consts::FRAC_1_SQRT_2);

fn along_z(d: anim_core::Vec2) -> f64 {
    d.y.mul_add(Z_DIRECTION.1, d.x * Z_DIRECTION.0)
}

fn grip_at(mode: Mode, centre: Point, p: Point, zoom: f64, tolerance: f64) -> Option<Grip> {
    let size = SIZE / zoom.max(1.0e-6);
    let d = p - centre;
    let radius = d.hypot();
    match mode {
        Mode::Translate => {
            if d.y.abs() <= tolerance && d.x >= -tolerance && d.x <= size {
                return Some(Grip::X);
            }
            if d.x.abs() <= tolerance && d.y >= -tolerance && d.y <= size {
                return Some(Grip::Y);
            }
            let along = along_z(d);
            let across = d.y.mul_add(-Z_DIRECTION.0, d.x * Z_DIRECTION.1).abs();
            (across <= tolerance && along >= -tolerance && along <= size * 0.7).then_some(Grip::Z)
        }
        Mode::Rotate => {
            if d.y.abs() <= tolerance && d.x.abs() <= size {
                return Some(Grip::X);
            }
            if d.x.abs() <= tolerance && d.y.abs() <= size {
                return Some(Grip::Y);
            }
            if (radius - size).abs() <= tolerance {
                return Some(Grip::Z);
            }
            (radius > size + tolerance && radius <= size * OUTER).then_some(Grip::Free)
        }
    }
}

fn snap(degrees: f64, on: bool) -> f64 {
    if on { (degrees / SNAP).round() * SNAP } else { degrees }
}

/// The stage-space matrix the drag produces.
fn delta(mode: Mode, drag: &Drag, zoom: f64) -> Matrix3D {
    let d = drag.current - drag.start;
    let shift = drag.modifiers.shift;
    let centre = at(drag.centre);
    match mode {
        Mode::Translate => {
            let (x, y, z) = match drag.grip {
                Grip::X => (d.x, 0.0, 0.0),
                Grip::Y => (0.0, d.y, 0.0),
                Grip::Z | Grip::Free => (0.0, 0.0, along_z(d)),
            };
            Matrix3D::translation(x, y, z)
        }
        Mode::Rotate => {
            let about_x = |dy: f64| Matrix3D::rotation_x(snap(-dy * zoom * DEGREES_PER_POINT, shift));
            let about_y = |dx: f64| Matrix3D::rotation_y(snap(-dx * zoom * DEGREES_PER_POINT, shift));
            let rotation = match drag.grip {
                Grip::X => about_x(d.y),
                Grip::Y => about_y(d.x),
                Grip::Z => {
                    let a0 = (drag.start - centre).atan2();
                    let a1 = (drag.current - centre).atan2();
                    Matrix3D::rotation_z(snap((a1 - a0).to_degrees(), shift))
                }
                Grip::Free => about_x(d.y).then(&about_y(d.x)),
            };
            Matrix3D::translation(-drag.centre[0], -drag.centre[1], -drag.centre[2]).then(&rotation).then(&Matrix3D::translation(
                drag.centre[0],
                drag.centre[1],
                drag.centre[2],
            ))
        }
    }
}

const fn cursor(mode: Mode, grip: Option<Grip>) -> Cursor {
    match (mode, grip) {
        (_, None) => Cursor::Default,
        (Mode::Rotate, Some(_)) => Cursor::Rotate,
        (Mode::Translate, Some(_)) => Cursor::Move,
    }
}

impl Gizmo {
    const fn new(mode: Mode) -> Self {
        Self { mode, drag: None }
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    let Some(centre) = gizmo_centre(ctx) else { return ToolOutput::none() };
                    let Some(grip) = grip_at(self.mode, at(centre), p.pos, ctx.zoom, ctx.tolerance()) else { return ToolOutput::none() };
                    self.drag = Some(Drag { grip, start: p.pos, current: p.pos, modifiers: p.modifiers, centre });
                    ToolOutput::cursor(cursor(self.mode, Some(grip)))
                }
                PointerKind::Move => {
                    if let Some(drag) = &mut self.drag {
                        drag.current = p.pos;
                        drag.modifiers = p.modifiers;
                        return ToolOutput::cursor(cursor(self.mode, Some(drag.grip)));
                    }
                    ToolOutput::none()
                }
                PointerKind::Up => {
                    let Some(mut drag) = self.drag.take() else { return ToolOutput::none() };
                    drag.current = p.pos;
                    drag.modifiers = p.modifiers;
                    let matrix = delta(self.mode, &drag, ctx.zoom);
                    let mut commands = Vec::new();
                    if matrix != Matrix3D::IDENTITY && matrix.is_finite() {
                        commands.push(ToolCommand::Transform3DSelection { matrix });
                    }
                    ToolOutput::with(cursor(self.mode, Some(drag.grip)), commands)
                }
                PointerKind::Hover => {
                    let grip = gizmo_centre(ctx).and_then(|c| grip_at(self.mode, at(c), p.pos, ctx.zoom, ctx.tolerance()));
                    ToolOutput::cursor(cursor(self.mode, grip))
                }
            },
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.drag = None;
                ToolOutput::none()
            }
            ToolEvent::Cancel => {
                self.drag = None;
                ToolOutput::none()
            }
            _ => ToolOutput::none(),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let Some(centre3) = gizmo_centre(ctx) else { return Vec::new() };
        let centre = at(centre3);
        let size = SIZE / ctx.zoom.max(1.0e-6);
        let mut out = Vec::new();
        match self.mode {
            Mode::Translate => {
                out.push(Overlay::Line { a: centre, b: centre + anim_core::Vec2::new(size, 0.0), role: Role::Handle });
                out.push(Overlay::Line { a: centre, b: centre + anim_core::Vec2::new(0.0, size), role: Role::Handle });
                out.push(Overlay::Line { a: centre, b: centre + anim_core::Vec2::new(Z_DIRECTION.0, Z_DIRECTION.1) * (size * 0.7), role: Role::Handle });
                out.push(Overlay::Marker { pos: centre + anim_core::Vec2::new(size, 0.0), marker: Marker::Diamond, role: Role::Handle, filled: true });
                out.push(Overlay::Marker { pos: centre + anim_core::Vec2::new(0.0, size), marker: Marker::Diamond, role: Role::Handle, filled: true });
                out.push(Overlay::Marker {
                    pos: centre + anim_core::Vec2::new(Z_DIRECTION.0, Z_DIRECTION.1) * (size * 0.7),
                    marker: Marker::Diamond,
                    role: Role::Handle,
                    filled: true,
                });
            }
            Mode::Rotate => {
                out.push(Overlay::Line { a: centre - anim_core::Vec2::new(size, 0.0), b: centre + anim_core::Vec2::new(size, 0.0), role: Role::Handle });
                out.push(Overlay::Line { a: centre - anim_core::Vec2::new(0.0, size), b: centre + anim_core::Vec2::new(0.0, size), role: Role::Handle });
                out.push(Overlay::Path { path: Circle::new(centre, size).to_path(0.1), role: Role::Handle, width: None, filled: false });
                out.push(Overlay::Path { path: Circle::new(centre, size * OUTER).to_path(0.1), role: Role::Guide, width: None, filled: false });
            }
        }
        out.push(Overlay::Marker { pos: centre, marker: Marker::Circle, role: Role::Pivot, filled: true });
        if let Some(drag) = &self.drag {
            let matrix = delta(self.mode, drag, ctx.zoom);
            let perspective = ctx.document.perspective();
            for resolved in instances(ctx) {
                let Some(instance) = instance_of(&resolved) else { continue };
                let Some(local) = ctx.bounds.local_bounds(resolved.element) else { continue };
                let to_stage = Matrix3D::from_affine(resolved.parent_to_stage);
                let Some(back) = invert(resolved.parent_to_stage) else { continue };
                let placement = instance.placement_3d().then(&to_stage).then(&matrix).then(&Matrix3D::from_affine(back));
                let homography = placement.homography(&perspective);
                let corners = [Point::new(local.x0, local.y0), Point::new(local.x1, local.y0), Point::new(local.x1, local.y1), Point::new(local.x0, local.y1)];
                let projected: Vec<Point> = corners.iter().filter_map(|c| homography.apply(*c)).map(|p| resolved.parent_to_stage * p).collect();
                if projected.len() == corners.len() {
                    out.push(Overlay::Polyline { points: projected, closed: true, role: Role::Preview });
                }
            }
        }
        out
    }
}

fn invert(a: Affine) -> Option<Affine> {
    let det = a.determinant();
    (det.is_finite() && det.abs() > 1.0e-12).then(|| a.inverse())
}

/// The 3D Rotation tool.
#[derive(Debug, Clone, Copy)]
pub struct Rotation3DTool(Gizmo);

impl Default for Rotation3DTool {
    fn default() -> Self {
        Self(Gizmo::new(Mode::Rotate))
    }
}

/// The 3D Translation tool.
#[derive(Debug, Clone, Copy)]
pub struct Translation3DTool(Gizmo);

impl Default for Translation3DTool {
    fn default() -> Self {
        Self(Gizmo::new(Mode::Translate))
    }
}

macro_rules! gizmo_tool {
    ($tool:ty, $id:expr) => {
        impl Tool for $tool {
            fn id(&self) -> ToolId {
                $id
            }

            fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
                self.0.handle(event, ctx)
            }

            fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
                self.0.overlay(ctx)
            }

            fn cancel(&mut self) {
                self.0.drag = None;
            }

            fn is_busy(&self) -> bool {
                self.0.drag.is_some()
            }
        }
    };
}

gizmo_tool!(Rotation3DTool, ToolId::Rotation3D);
gizmo_tool!(Translation3DTool, ToolId::Translation3D);

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(grip: Grip, start: (f64, f64), end: (f64, f64)) -> Drag {
        Drag { grip, start: Point::new(start.0, start.1), current: Point::new(end.0, end.1), modifiers: Modifiers::NONE, centre: [100.0, 100.0, 0.0] }
    }

    #[test]
    fn grips_are_found_on_the_axes_and_rings() {
        let c = Point::new(100.0, 100.0);
        assert_eq!(grip_at(Mode::Translate, c, Point::new(130.0, 100.0), 1.0, 4.0), Some(Grip::X));
        assert_eq!(grip_at(Mode::Translate, c, Point::new(100.0, 130.0), 1.0, 4.0), Some(Grip::Y));
        assert_eq!(grip_at(Mode::Translate, c, Point::new(120.0, 80.0), 1.0, 4.0), Some(Grip::Z));
        assert_eq!(grip_at(Mode::Translate, c, Point::new(130.0, 130.0), 1.0, 4.0), None);
        assert_eq!(grip_at(Mode::Rotate, c, Point::new(70.0, 100.0), 1.0, 4.0), Some(Grip::X));
        assert_eq!(grip_at(Mode::Rotate, c, Point::new(100.0, 40.0), 1.0, 4.0), Some(Grip::Y));
        let on_circle = 100.0 + SIZE * std::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(grip_at(Mode::Rotate, c, Point::new(on_circle, on_circle), 1.0, 4.0), Some(Grip::Z));
        assert_eq!(grip_at(Mode::Rotate, c, Point::new(100.0 + SIZE * 1.15, 100.0 + SIZE * 0.2), 1.0, 4.0), Some(Grip::Free));
    }

    #[test]
    fn translation_moves_along_one_axis() {
        let m = delta(Mode::Translate, &drag(Grip::X, (130.0, 100.0), (150.0, 140.0)), 1.0);
        assert_eq!(m.transform_point([0.0, 0.0, 0.0]), [20.0, 0.0, 0.0]);
        let m = delta(Mode::Translate, &drag(Grip::Y, (100.0, 130.0), (110.0, 145.0)), 1.0);
        assert_eq!(m.transform_point([0.0, 0.0, 0.0]), [0.0, 15.0, 0.0]);
        let m = delta(Mode::Translate, &drag(Grip::Z, (0.0, 0.0), (10.0, -10.0)), 1.0);
        let z = m.transform_point([0.0, 0.0, 0.0])[2];
        assert!((z - 10.0 * 2.0_f64.sqrt()).abs() < 1.0e-9, "{z}");
    }

    #[test]
    fn rotation_about_y_keeps_the_centre_and_turns_the_front_with_the_drag() {
        // Dragging 45 screen points to the right turns 45 degrees about the vertical axis through the centre.
        let m = delta(Mode::Rotate, &drag(Grip::Y, (100.0, 60.0), (145.0, 60.0)), 1.0);
        let centre = m.transform_point([100.0, 100.0, 0.0]);
        assert!(centre.iter().zip([100.0, 100.0, 0.0]).all(|(a, b)| (a - b).abs() < 1.0e-9), "{centre:?}");
        // The right edge goes away from the viewer (positive z) and in toward the centre.
        let right = m.transform_point([200.0, 100.0, 0.0]);
        assert!((right[0] - (100.0 + 100.0 * 45.0_f64.to_radians().cos())).abs() < 1.0e-9, "{right:?}");
        assert!(right[2] > 0.0, "{right:?}");
    }

    #[test]
    fn rotation_about_z_follows_the_pointer_around_the_centre() {
        let m = delta(Mode::Rotate, &drag(Grip::Z, (170.0, 100.0), (100.0, 170.0)), 1.0);
        let p = m.transform_point([170.0, 100.0, 0.0]);
        assert!((p[0] - 100.0).abs() < 1.0e-9 && (p[1] - 170.0).abs() < 1.0e-9, "{p:?}");
    }

    #[test]
    fn shift_snaps_rotation_to_fifteen_degrees() {
        let mut d = drag(Grip::Y, (0.0, 0.0), (-40.0, 0.0));
        d.modifiers = Modifiers::SHIFT;
        let m = delta(Mode::Rotate, &d, 1.0);
        let expect = Matrix3D::translation(-100.0, -100.0, 0.0).then(&Matrix3D::rotation_y(45.0)).then(&Matrix3D::translation(100.0, 100.0, 0.0));
        for (a, b) in m.values().iter().zip(expect.values()) {
            assert!((a - b).abs() < 1.0e-9);
        }
    }
}
