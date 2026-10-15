//! The Free Transform tool: scale, rotate and skew the selection around the transformation point.

use anim_core::{Affine, Point, Rect};
use anim_scene::Element;

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, Modifiers, PointerEvent, PointerKind, ToolEvent};
use crate::overlay::{Compass, Cursor, Marker, Overlay, Role};
use crate::selection::ElementRef;
use crate::tool::{Tool, ToolId, ToolOutput};

/// The eight scale handles as `(dx, dy)` directions, clockwise from the top left.
const DIRS: [(i8, i8); 8] = [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)];
/// Handle hit radius in screen points.
const HANDLE_RADIUS: f64 = 6.0;
/// How far outside a corner the rotate zone reaches, in screen points.
const ROTATE_REACH: f64 = 20.0;
/// Rotation snap with shift, in radians.
const ROTATE_SNAP: f64 = std::f64::consts::PI / 12.0;

/// What the pointer grabbed. Directions are -1, 0 or 1 on each axis (x right, y down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grip {
    Scale(i8, i8),
    Rotate,
    /// Skew by dragging an edge: the edge is `(dx, dy)` with exactly one non-zero component.
    Skew(i8, i8),
    Move,
    Pivot,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    grip: Grip,
    start: Point,
    current: Point,
    modifiers: Modifiers,
    bounds: Rect,
    pivot: Point,
}

/// Scales, rotates and skews the selection.
#[derive(Debug, Clone, Default)]
pub struct FreeTransformTool {
    drag: Option<Drag>,
    /// The transformation point, kept while the selection stays the same.
    pivot: Option<(Vec<ElementRef>, Point)>,
}

fn direction_point(bounds: Rect, dx: i8, dy: i8) -> Point {
    let c = bounds.center();
    Point::new(c.x + f64::from(dx) * bounds.width() / 2.0, c.y + f64::from(dy) * bounds.height() / 2.0)
}

const fn compass(dx: i8, dy: i8) -> Compass {
    match (dx, dy) {
        (0, -1) => Compass::N,
        (1, -1) => Compass::Ne,
        (1, 0) => Compass::E,
        (1, 1) => Compass::Se,
        (0, 1) => Compass::S,
        (-1, 1) => Compass::Sw,
        (-1, 0) => Compass::W,
        _ => Compass::Nw,
    }
}

fn safe_ratio(numerator: f64, denominator: f64) -> f64 {
    if denominator.abs() < 1.0e-9 { 1.0 } else { numerator / denominator }
}

/// Matrix for dragging the scale handle `(dx, dy)` of `bounds` to `pointer`. The opposite side stays put, or the
/// pivot with alt; shift keeps the proportions.
fn scale_matrix(bounds: Rect, dx: i8, dy: i8, pointer: Point, pivot: Point, modifiers: Modifiers) -> Affine {
    let handle = direction_point(bounds, dx, dy);
    let anchor = if modifiers.alt { pivot } else { direction_point(bounds, dx.saturating_neg(), dy.saturating_neg()) };
    let mut sx = if dx == 0 { 1.0 } else { safe_ratio(pointer.x - anchor.x, handle.x - anchor.x) };
    let mut sy = if dy == 0 { 1.0 } else { safe_ratio(pointer.y - anchor.y, handle.y - anchor.y) };
    if modifiers.shift {
        let dominant = match (dx, dy) {
            (0, _) => sy,
            (_, 0) => sx,
            _ if sx.abs() > sy.abs() => sx,
            _ => sy,
        };
        // Same magnitude on both axes; each keeps its own direction so a flip past the anchor still works.
        let magnitude = dominant.abs();
        sx = magnitude.copysign(if dx == 0 { dominant } else { sx });
        sy = magnitude.copysign(if dy == 0 { dominant } else { sy });
    }
    Affine::translate(anchor.to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-anchor.to_vec2())
}

fn rotate_matrix(pivot: Point, start: Point, current: Point, snap: bool) -> Affine {
    let a0 = (start - pivot).atan2();
    let a1 = (current - pivot).atan2();
    let mut angle = a1 - a0;
    if snap {
        angle = (angle / ROTATE_SNAP).round() * ROTATE_SNAP;
    }
    Affine::translate(pivot.to_vec2()) * Affine::rotate(angle) * Affine::translate(-pivot.to_vec2())
}

fn skew_matrix(bounds: Rect, dx: i8, dy: i8, start: Point, current: Point) -> Affine {
    if dy != 0 {
        // Top or bottom edge: slide it horizontally, the opposite edge stays.
        let fixed_y = direction_point(bounds, 0, dy.saturating_neg()).y;
        let k = safe_ratio(current.x - start.x, direction_point(bounds, 0, dy).y - fixed_y);
        Affine::new([1.0, 0.0, k, 1.0, -k * fixed_y, 0.0])
    } else {
        let fixed_x = direction_point(bounds, dx.saturating_neg(), 0).x;
        let k = safe_ratio(current.y - start.y, direction_point(bounds, dx, 0).x - fixed_x);
        Affine::new([1.0, k, 0.0, 1.0, 0.0, -k * fixed_x])
    }
}

fn move_matrix(start: Point, current: Point, modifiers: Modifiers) -> Affine {
    let mut d = current - start;
    if modifiers.shift {
        if d.x.abs() >= d.y.abs() {
            d.y = 0.0;
        } else {
            d.x = 0.0;
        }
    }
    Affine::translate(d)
}

fn drag_matrix(d: &Drag) -> Affine {
    match d.grip {
        Grip::Scale(dx, dy) => scale_matrix(d.bounds, dx, dy, d.current, d.pivot, d.modifiers),
        Grip::Rotate => rotate_matrix(d.pivot, d.start, d.current, d.modifiers.shift),
        Grip::Skew(dx, dy) => skew_matrix(d.bounds, dx, dy, d.start, d.current),
        Grip::Move => move_matrix(d.start, d.current, d.modifiers),
        Grip::Pivot => Affine::IDENTITY,
    }
}

fn grip_at(bounds: Rect, pivot: Point, p: Point, zoom: f64) -> Option<Grip> {
    let z = zoom.max(1.0e-6);
    let radius = HANDLE_RADIUS / z;
    if pivot.distance(p) <= radius {
        return Some(Grip::Pivot);
    }
    let mut best: Option<(f64, i8, i8)> = None;
    for (dx, dy) in DIRS {
        let d = direction_point(bounds, dx, dy).distance(p);
        if d <= radius && best.is_none_or(|(bd, ..)| d < bd) {
            best = Some((d, dx, dy));
        }
    }
    if let Some((_, dx, dy)) = best {
        return Some(Grip::Scale(dx, dy));
    }
    let inside = p.x >= bounds.x0 && p.x <= bounds.x1 && p.y >= bounds.y0 && p.y <= bounds.y1;
    if !inside {
        let reach = ROTATE_REACH / z;
        let near_corner = [(-1, -1), (1, -1), (1, 1), (-1, 1)].iter().any(|&(dx, dy)| direction_point(bounds, dx, dy).distance(p) <= reach);
        // Along an edge, outside the box by less than the handle radius, is a skew.
        let near_x = p.x >= bounds.x0 && p.x <= bounds.x1;
        let near_y = p.y >= bounds.y0 && p.y <= bounds.y1;
        if near_x && (p.y - bounds.y0).abs() <= radius {
            return Some(Grip::Skew(0, -1));
        }
        if near_x && (p.y - bounds.y1).abs() <= radius {
            return Some(Grip::Skew(0, 1));
        }
        if near_y && (p.x - bounds.x0).abs() <= radius {
            return Some(Grip::Skew(-1, 0));
        }
        if near_y && (p.x - bounds.x1).abs() <= radius {
            return Some(Grip::Skew(1, 0));
        }
        if near_corner {
            return Some(Grip::Rotate);
        }
        return None;
    }
    // Inside the box, close to an edge is a skew too; elsewhere it moves.
    let edge = radius / 2.0;
    if (p.y - bounds.y0).abs() <= edge {
        return Some(Grip::Skew(0, -1));
    }
    if (p.y - bounds.y1).abs() <= edge {
        return Some(Grip::Skew(0, 1));
    }
    if (p.x - bounds.x0).abs() <= edge {
        return Some(Grip::Skew(-1, 0));
    }
    if (p.x - bounds.x1).abs() <= edge {
        return Some(Grip::Skew(1, 0));
    }
    Some(Grip::Move)
}

const fn grip_cursor(grip: Option<Grip>) -> Cursor {
    match grip {
        None => Cursor::Default,
        Some(Grip::Scale(dx, dy)) => Cursor::Resize(compass(dx, dy)),
        Some(Grip::Rotate) => Cursor::Rotate,
        Some(Grip::Skew(..)) => Cursor::Skew,
        Some(Grip::Move) => Cursor::Move,
        Some(Grip::Pivot) => Cursor::Pivot,
    }
}

impl FreeTransformTool {
    /// The transformation point for the current selection: the remembered one, a single element's own point, or the
    /// centre of the bounds.
    fn pivot_for(&self, ctx: &ToolContext<'_>, bounds: Rect) -> Point {
        if let Some((items, p)) = &self.pivot
            && *items == ctx.selection.items
        {
            return *p;
        }
        if let [only] = ctx.selection.items.as_slice()
            && let Some(resolved) = ctx.resolve(only)
        {
            let local = match resolved.element {
                Element::Shape(s) => s.transformation_point,
                Element::Group(g) => g.transformation_point,
                Element::SymbolInstance(i) => Some(i.transformation_point),
                _ => None,
            };
            if let Some(local) = local {
                let stage = resolved.local_to_stage() * local;
                if stage.x.is_finite() && stage.y.is_finite() {
                    return stage;
                }
            }
        }
        bounds.center()
    }

    fn down(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let Some(bounds) = ctx.selection_bounds() else { return ToolOutput::none() };
        let pivot = self.pivot_for(ctx, bounds);
        let Some(grip) = grip_at(bounds, pivot, p.pos, ctx.zoom) else { return ToolOutput::none() };
        self.drag = Some(Drag { grip, start: p.pos, current: p.pos, modifiers: p.modifiers, bounds, pivot });
        ToolOutput::cursor(grip_cursor(Some(grip)))
    }

    fn up(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let Some(mut drag) = self.drag.take() else { return ToolOutput::none() };
        drag.current = p.pos;
        drag.modifiers = p.modifiers;
        let mut commands = Vec::new();
        if drag.grip == Grip::Pivot {
            self.pivot = Some((ctx.selection.items.clone(), p.pos));
            if let [only] = ctx.selection.items.as_slice() {
                commands.push(ToolCommand::SetTransformationPoint { target: only.clone(), point: p.pos });
            }
        } else {
            let matrix = drag_matrix(&drag);
            if matrix != Affine::IDENTITY && matrix.as_coeffs().iter().all(|c| c.is_finite()) {
                // The pivot follows the transformation, like the objects.
                self.pivot = Some((ctx.selection.items.clone(), matrix * drag.pivot));
                commands.push(ToolCommand::TransformSelection { matrix });
            }
        }
        ToolOutput::with(grip_cursor(Some(drag.grip)), commands)
    }
}

impl Tool for FreeTransformTool {
    fn id(&self) -> ToolId {
        ToolId::FreeTransform
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => self.down(p, ctx),
                PointerKind::Move => {
                    if let Some(d) = &mut self.drag {
                        d.current = p.pos;
                        d.modifiers = p.modifiers;
                        return ToolOutput::cursor(grip_cursor(Some(d.grip)));
                    }
                    ToolOutput::none()
                }
                PointerKind::Up => self.up(p, ctx),
                PointerKind::Hover => {
                    let grip = ctx.selection_bounds().and_then(|b| grip_at(b, self.pivot_for(ctx, b), p.pos, ctx.zoom));
                    ToolOutput::cursor(grip_cursor(grip))
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
        let Some(bounds) = ctx.selection_bounds() else { return Vec::new() };
        let pivot = self.pivot_for(ctx, bounds);
        let mut out = vec![Overlay::Rect { rect: bounds, role: Role::Selection }];
        for (dx, dy) in DIRS {
            out.push(Overlay::Marker { pos: direction_point(bounds, dx, dy), marker: Marker::Square, role: Role::Handle, filled: false });
        }
        let shown_pivot = match &self.drag {
            Some(d) if d.grip == Grip::Pivot => d.current,
            Some(d) => drag_matrix(d) * pivot,
            None => pivot,
        };
        out.push(Overlay::Marker { pos: shown_pivot, marker: Marker::Circle, role: Role::Pivot, filled: false });
        out.push(Overlay::Crosshair { pos: shown_pivot, role: Role::Pivot });
        if let Some(d) = &self.drag
            && d.grip != Grip::Pivot
        {
            let m = drag_matrix(d);
            let corners =
                [Point::new(bounds.x0, bounds.y0), Point::new(bounds.x1, bounds.y0), Point::new(bounds.x1, bounds.y1), Point::new(bounds.x0, bounds.y1)];
            out.push(Overlay::Polyline { points: corners.iter().map(|c| m * *c).collect(), closed: true, role: Role::Preview });
        }
        out
    }

    fn cancel(&mut self) {
        self.drag = None;
    }

    fn is_busy(&self) -> bool {
        self.drag.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Rect {
        Rect::new(100.0, 100.0, 200.0, 160.0)
    }

    fn close(a: Point, b: Point) -> bool {
        a.distance(b) < 1.0e-9
    }

    #[test]
    fn corner_scale_keeps_the_opposite_corner() {
        let m = scale_matrix(r(), 1, 1, Point::new(300.0, 220.0), Point::new(150.0, 130.0), Modifiers::NONE);
        assert!(close(m * Point::new(100.0, 100.0), Point::new(100.0, 100.0)));
        assert!(close(m * Point::new(200.0, 160.0), Point::new(300.0, 220.0)));
    }

    #[test]
    fn alt_scales_about_the_pivot() {
        let pivot = Point::new(150.0, 130.0);
        let m = scale_matrix(r(), 1, 1, Point::new(250.0, 190.0), pivot, Modifiers::ALT);
        assert!(close(m * pivot, pivot));
        assert!(close(m * Point::new(200.0, 160.0), Point::new(250.0, 190.0)));
    }

    #[test]
    fn shift_scale_is_uniform() {
        let m = scale_matrix(r(), 1, 1, Point::new(300.0, 130.0), Point::new(150.0, 130.0), Modifiers::SHIFT);
        let coeffs = m.as_coeffs();
        assert!((coeffs[0] - coeffs[3]).abs() < 1.0e-9 && coeffs[1].abs() < 1.0e-9 && coeffs[2].abs() < 1.0e-9);
    }

    #[test]
    fn edge_handle_scales_one_axis() {
        let m = scale_matrix(r(), 1, 0, Point::new(300.0, 500.0), Point::new(150.0, 130.0), Modifiers::NONE);
        assert!(close(m * Point::new(200.0, 160.0), Point::new(300.0, 160.0)));
        assert!(close(m * Point::new(100.0, 100.0), Point::new(100.0, 100.0)));
    }

    #[test]
    fn rotation_turns_a_quarter_about_the_pivot() {
        let pivot = Point::new(0.0, 0.0);
        let m = rotate_matrix(pivot, Point::new(10.0, 0.0), Point::new(0.0, 10.0), false);
        assert!(close(m * Point::new(10.0, 0.0), Point::new(0.0, 10.0)));
        let snapped = rotate_matrix(pivot, Point::new(10.0, 0.0), Point::new(10.0, 1.0), true);
        assert!(close(snapped * Point::new(10.0, 0.0), Point::new(10.0, 0.0)));
    }

    #[test]
    fn skew_moves_the_dragged_edge_and_pins_the_opposite_one() {
        let m = skew_matrix(r(), 0, -1, Point::new(150.0, 100.0), Point::new(180.0, 100.0));
        assert!(close(m * Point::new(120.0, 100.0), Point::new(150.0, 100.0)));
        assert!(close(m * Point::new(120.0, 160.0), Point::new(120.0, 160.0)));
    }

    #[test]
    fn grips_are_found_by_position() {
        let b = r();
        let pivot = b.center();
        assert_eq!(grip_at(b, pivot, Point::new(100.0, 100.0), 1.0), Some(Grip::Scale(-1, -1)));
        assert_eq!(grip_at(b, pivot, Point::new(150.0, 100.0), 1.0), Some(Grip::Scale(0, -1)));
        assert_eq!(grip_at(b, pivot, pivot, 1.0), Some(Grip::Pivot));
        assert_eq!(grip_at(b, pivot, Point::new(120.0, 140.0), 1.0), Some(Grip::Move));
        assert_eq!(grip_at(b, pivot, Point::new(210.0, 170.0), 1.0), Some(Grip::Rotate));
        assert_eq!(grip_at(b, pivot, Point::new(130.0, 97.0), 1.0), Some(Grip::Skew(0, -1)));
        assert_eq!(grip_at(b, pivot, Point::new(400.0, 400.0), 1.0), None);
    }
}
