//! Line, Rectangle and Oval: drag a gesture, get one shape.

use anim_core::kurbo::{Arc, Ellipse, PathEl, RoundedRect, RoundedRectRadii, Shape as _};
use anim_core::{BezPath, Point, Vec2};
use anim_shape::{EdgeShape, PathInput};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, Modifiers, PointerKind, ToolEvent};
use crate::overlay::{Cursor, Overlay, Role};
use crate::settings::{OvalSettings, ToolSettings};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Smallest drag (stage pixels) that still makes a shape.
const MIN_SIZE: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Line,
    Rectangle,
    Oval,
}

impl Kind {
    const fn id(self) -> ToolId {
        match self {
            Self::Line => ToolId::Line,
            Self::Rectangle => ToolId::Rectangle,
            Self::Oval => ToolId::Oval,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Gesture {
    start: Point,
    current: Point,
    modifiers: Modifiers,
}

/// The two corners a drag defines once shift (square) and alt (from centre) are applied.
fn corners(g: &Gesture) -> (Point, Point) {
    let mut d = g.current - g.start;
    if g.modifiers.shift {
        let side = d.x.abs().max(d.y.abs());
        d = Vec2::new(side.copysign(if d.x == 0.0 { 1.0 } else { d.x }), side.copysign(if d.y == 0.0 { 1.0 } else { d.y }));
    }
    if g.modifiers.alt { (g.start - d, g.start + d) } else { (g.start, g.start + d) }
}

/// The end of a line drag: with shift the direction snaps to multiples of 45 degrees.
fn line_ends(g: &Gesture) -> (Point, Point) {
    let mut d = g.current - g.start;
    if g.modifiers.shift {
        let len = d.hypot();
        let step = std::f64::consts::FRAC_PI_4;
        let angle = (d.y.atan2(d.x) / step).round() * step;
        d = Vec2::new(len * angle.cos(), len * angle.sin());
    }
    if g.modifiers.alt { (g.start - d, g.start + d) } else { (g.start, g.start + d) }
}

/// The rectangle with the given corner radii (top left, top right, bottom right, bottom left), each limited to
/// half the shorter side.
#[must_use]
pub fn rounded_rect_path(rect: anim_core::Rect, radii: [f64; 4]) -> BezPath {
    let limit = rect.width().min(rect.height()) / 2.0;
    let [tl, tr, br, bl] = radii.map(|r| if r.is_finite() { r.max(0.0).min(limit) } else { 0.0 });
    if tl + tr + br + bl > 0.0 { RoundedRect::from_rect(rect, RoundedRectRadii::new(tl, tr, br, bl)).to_path(0.1) } else { rect.to_path(0.1) }
}

/// An ellipse, or the arc, pie or ring segment the oval settings describe, inside `rect`.
#[must_use]
pub fn oval_path(rect: anim_core::Rect, oval: &OvalSettings) -> BezPath {
    let centre = rect.center();
    let radii = Vec2::new(rect.width() / 2.0, rect.height() / 2.0);
    if oval.is_plain() {
        return Ellipse::new(centre, radii, 0.0).to_path(0.1);
    }
    let start = oval.start_angle.to_radians();
    let mut sweep = (oval.end_angle - oval.start_angle).rem_euclid(360.0).to_radians();
    if sweep <= 0.0 {
        sweep = std::f64::consts::TAU;
    }
    let inner = (oval.inner_radius / 100.0).clamp(0.0, 0.99);
    let mut path = BezPath::new();
    for el in Arc::new(centre, radii, start, sweep, 0.0).path_elements(0.05) {
        path.push(el);
    }
    if inner > 0.0 {
        let mut tail = Arc::new(centre, radii * inner, start + sweep, -sweep, 0.0).path_elements(0.05);
        if let Some(PathEl::MoveTo(p)) = tail.next() {
            path.line_to(p);
        }
        for el in tail {
            path.push(el);
        }
        path.close_path();
    } else if oval.close_path {
        path.line_to(centre);
        path.close_path();
    }
    path
}

fn gesture_path(kind: Kind, g: &Gesture, settings: &ToolSettings) -> BezPath {
    match kind {
        Kind::Line => {
            let (a, b) = line_ends(g);
            let mut p = BezPath::new();
            p.move_to(a);
            p.line_to(b);
            p
        }
        Kind::Rectangle => {
            let (a, b) = corners(g);
            rounded_rect_path(anim_core::Rect::from_points(a, b), settings.radii())
        }
        Kind::Oval => {
            let (a, b) = corners(g);
            oval_path(anim_core::Rect::from_points(a, b), &settings.oval)
        }
    }
}

/// Shared state machine of the three tools.
#[derive(Debug, Clone)]
struct ShapeDrag {
    kind: Kind,
    gesture: Option<Gesture>,
}

impl ShapeDrag {
    const fn new(kind: Kind) -> Self {
        Self { kind, gesture: None }
    }

    fn build(&self, g: &Gesture, ctx: &ToolContext<'_>) -> Option<(BezPath, EdgeShape)> {
        let path = gesture_path(self.kind, g, ctx.settings);
        let (fill, stroke) = match self.kind {
            Kind::Line => (None, Some(ctx.settings.stroke.clone())),
            Kind::Rectangle | Kind::Oval => {
                let bounds = path.bounding_box();
                (ctx.settings.active_fill().map(|f| crate::fillfit::fit_fill(&f, bounds)), ctx.settings.active_stroke())
            }
        };
        if fill.is_none() && stroke.is_none() {
            return None;
        }
        let shape = EdgeShape::from_paths(&[PathInput::new(path.clone(), fill, stroke)]);
        (!shape.is_empty()).then_some((path, shape))
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    self.gesture = Some(Gesture { start: p.pos, current: p.pos, modifiers: p.modifiers });
                    ToolOutput::cursor(Cursor::Crosshair)
                }
                PointerKind::Move => {
                    if let Some(g) = &mut self.gesture {
                        g.current = p.pos;
                        g.modifiers = p.modifiers;
                    }
                    ToolOutput::cursor(Cursor::Crosshair)
                }
                PointerKind::Up => {
                    let Some(mut g) = self.gesture.take() else { return ToolOutput::cursor(Cursor::Crosshair) };
                    g.current = p.pos;
                    g.modifiers = p.modifiers;
                    let (a, b) = if self.kind == Kind::Line { line_ends(&g) } else { corners(&g) };
                    let big_enough =
                        if self.kind == Kind::Line { a.distance(b) >= MIN_SIZE } else { (b.x - a.x).abs() >= MIN_SIZE && (b.y - a.y).abs() >= MIN_SIZE };
                    if !big_enough || !ctx.layer_editable(ctx.layer) {
                        return ToolOutput::cursor(Cursor::Crosshair);
                    }
                    match self.build(&g, ctx) {
                        Some((_, shape)) => ToolOutput::with(
                            Cursor::Crosshair,
                            vec![ToolCommand::DrawShape { layer: ctx.layer, frame: ctx.frame, shape, object_drawing: ctx.settings.object_drawing }],
                        ),
                        None => ToolOutput::cursor(Cursor::Crosshair),
                    }
                }
                PointerKind::Hover => ToolOutput::cursor(Cursor::Crosshair),
            },
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.gesture = None;
                ToolOutput::cursor(Cursor::Crosshair)
            }
            ToolEvent::Cancel => {
                self.gesture = None;
                ToolOutput::cursor(Cursor::Crosshair)
            }
            _ => ToolOutput::cursor(Cursor::Crosshair),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let Some(g) = &self.gesture else { return Vec::new() };
        let path = gesture_path(self.kind, g, ctx.settings);
        vec![Overlay::Path { path, role: Role::Preview, width: None, filled: false }]
    }
}

macro_rules! drawing_tool {
    ($name:ident, $kind:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone)]
        pub struct $name(ShapeDrag);

        impl Default for $name {
            fn default() -> Self {
                Self(ShapeDrag::new($kind))
            }
        }

        impl Tool for $name {
            fn id(&self) -> ToolId {
                $kind.id()
            }

            fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
                self.0.handle(event, ctx)
            }

            fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
                self.0.overlay(ctx)
            }

            fn cancel(&mut self) {
                self.0.gesture = None;
            }

            fn is_busy(&self) -> bool {
                self.0.gesture.is_some()
            }
        }
    };
}

drawing_tool!(LineTool, Kind::Line, "Draws a straight line with the current stroke.");
drawing_tool!(RectangleTool, Kind::Rectangle, "Draws a rectangle, rounded by the corner radius setting.");
drawing_tool!(OvalTool, Kind::Oval, "Draws an oval or, with shift, a circle.");

#[cfg(test)]
mod tests {
    use super::*;

    fn g(sx: f64, sy: f64, cx: f64, cy: f64, modifiers: Modifiers) -> Gesture {
        Gesture { start: Point::new(sx, sy), current: Point::new(cx, cy), modifiers }
    }

    #[test]
    fn shift_makes_a_square_in_the_drag_direction() {
        let (a, b) = corners(&g(10.0, 10.0, 30.0, -5.0, Modifiers::SHIFT));
        assert_eq!(a, Point::new(10.0, 10.0));
        assert_eq!(b, Point::new(30.0, -10.0));
    }

    #[test]
    fn alt_draws_from_the_centre() {
        let (a, b) = corners(&g(50.0, 50.0, 60.0, 70.0, Modifiers::ALT));
        assert_eq!(a, Point::new(40.0, 30.0));
        assert_eq!(b, Point::new(60.0, 70.0));
    }

    #[test]
    fn shift_snaps_lines_to_45_degrees() {
        let (_, b) = line_ends(&g(0.0, 0.0, 100.0, 12.0, Modifiers::SHIFT));
        assert!(b.y.abs() < 1.0e-9);
        assert!((b.x - 100.0_f64.hypot(12.0)).abs() < 1.0e-9);
        let (_, b) = line_ends(&g(0.0, 0.0, 50.0, 60.0, Modifiers::SHIFT));
        assert!((b.x - b.y).abs() < 1.0e-9);
    }
}
