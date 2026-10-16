//! Line, Rectangle, Oval, their primitive variants and `PolyStar`: drag a gesture, get one shape.

use anim_core::kurbo::Shape as _;
use anim_core::{Affine, BezPath, Point, Vec2};
use anim_scene::{Element, OvalPrimitive, Primitive, PrimitiveKind, RectPrimitive, ShapeElement};
use anim_shape::{EdgeShape, PathInput};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, Modifiers, PointerKind, ToolEvent};
use crate::overlay::{Cursor, Overlay, Role};
use crate::settings::{OvalSettings, PolyStarSettings, PolyStarStyle, ToolSettings};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Smallest drag (stage pixels) that still makes a shape.
const MIN_SIZE: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Line,
    Rectangle,
    Oval,
    RectanglePrimitive,
    OvalPrimitive,
    PolyStar,
}

impl Kind {
    const fn id(self) -> ToolId {
        match self {
            Self::Line => ToolId::Line,
            Self::Rectangle => ToolId::Rectangle,
            Self::Oval => ToolId::Oval,
            Self::RectanglePrimitive => ToolId::RectanglePrimitive,
            Self::OvalPrimitive => ToolId::OvalPrimitive,
            Self::PolyStar => ToolId::PolyStar,
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
    anim_scene::primitive::rounded_rect_path(rect, radii)
}

/// An ellipse, or the arc, pie or ring segment the oval settings describe, inside `rect`.
#[must_use]
pub fn oval_path(rect: anim_core::Rect, oval: &OvalSettings) -> BezPath {
    anim_scene::primitive::oval_path(rect, oval.start_angle, oval.end_angle, oval.inner_radius, oval.close_path)
}

/// A regular polygon, or a star, with its outer corners on a circle of radius `|to - centre|` around `centre`. The
/// first corner points toward `to` (snapped to 15 degrees with `snap`).
#[must_use]
pub fn polystar_path(centre: Point, to: Point, snap: bool, options: &PolyStarSettings) -> BezPath {
    let d = to - centre;
    let radius = d.hypot();
    let mut path = BezPath::new();
    if !radius.is_finite() || radius <= 0.0 {
        return path;
    }
    let mut angle = d.y.atan2(d.x);
    if snap {
        let step = std::f64::consts::PI / 12.0;
        angle = (angle / step).round() * step;
    }
    let sides = options.sides.clamp(3, 32);
    let step = std::f64::consts::TAU / f64::from(sides);
    let inner = radius * options.star_point_size.clamp(0.01, 1.0);
    let at = |r: f64, a: f64| Point::new(r.mul_add(a.cos(), centre.x), r.mul_add(a.sin(), centre.y));
    for i in 0..sides {
        let a = step.mul_add(f64::from(i), angle);
        if i == 0 {
            path.move_to(at(radius, a));
        } else {
            path.line_to(at(radius, a));
        }
        if options.style == PolyStarStyle::Star {
            path.line_to(at(inner, a + step / 2.0));
        }
    }
    path.close_path();
    path
}

/// The primitive a rectangle or oval drag makes, with the matrix that places it, in the toolbox styles.
fn primitive_for(kind: Kind, rect: anim_core::Rect, settings: &ToolSettings) -> Option<(Primitive, Affine)> {
    let (w, h) = (rect.width(), rect.height());
    let local = anim_core::Rect::new(0.0, 0.0, w, h);
    let shape_kind = match kind {
        Kind::RectanglePrimitive => PrimitiveKind::Rectangle(RectPrimitive { width: w, height: h, radii: settings.radii(), lock: settings.corner_lock }),
        Kind::OvalPrimitive => PrimitiveKind::Oval(OvalPrimitive {
            width: w,
            height: h,
            start_angle: settings.oval.start_angle,
            end_angle: settings.oval.end_angle,
            inner_radius: settings.oval.inner_radius,
            close_path: settings.oval.close_path,
        }),
        _ => return None,
    };
    let fill = settings.active_fill().map(|f| crate::fillfit::fit_fill(&f, local));
    let stroke = settings.active_stroke();
    if fill.is_none() && stroke.is_none() {
        return None;
    }
    Some((Primitive { kind: shape_kind, fill, stroke }, Affine::translate((rect.x0, rect.y0))))
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
        Kind::Rectangle | Kind::RectanglePrimitive => {
            let (a, b) = corners(g);
            rounded_rect_path(anim_core::Rect::from_points(a, b), settings.radii())
        }
        Kind::Oval | Kind::OvalPrimitive => {
            let (a, b) = corners(g);
            oval_path(anim_core::Rect::from_points(a, b), &settings.oval)
        }
        Kind::PolyStar => polystar_path(g.start, g.current, g.modifiers.shift, &settings.polystar),
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
            Kind::Rectangle | Kind::Oval | Kind::RectanglePrimitive | Kind::OvalPrimitive | Kind::PolyStar => {
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
                    let (a, b) = match self.kind {
                        Kind::Line => line_ends(&g),
                        Kind::PolyStar => (g.start, g.current),
                        _ => corners(&g),
                    };
                    let big_enough = match self.kind {
                        Kind::Line | Kind::PolyStar => a.distance(b) >= MIN_SIZE,
                        _ => (b.x - a.x).abs() >= MIN_SIZE && (b.y - a.y).abs() >= MIN_SIZE,
                    };
                    if !big_enough || !ctx.layer_editable(ctx.layer) {
                        return ToolOutput::cursor(Cursor::Crosshair);
                    }
                    if matches!(self.kind, Kind::RectanglePrimitive | Kind::OvalPrimitive) {
                        let Some((primitive, matrix)) = primitive_for(self.kind, anim_core::Rect::from_points(a, b), ctx.settings) else {
                            return ToolOutput::cursor(Cursor::Crosshair);
                        };
                        let element = Element::Shape(ShapeElement::from_primitive(primitive, matrix));
                        return ToolOutput::with(Cursor::Crosshair, vec![ToolCommand::AddElement { layer: ctx.layer, frame: ctx.frame, element }]);
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
drawing_tool!(RectanglePrimitiveTool, Kind::RectanglePrimitive, "Draws a rectangle primitive whose size and corner radii stay editable.");
drawing_tool!(OvalPrimitiveTool, Kind::OvalPrimitive, "Draws an oval primitive whose angles and inner radius stay editable.");
drawing_tool!(PolyStarTool, Kind::PolyStar, "Draws a polygon or a star from its centre.");

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
    fn a_polygon_has_one_corner_per_side_and_a_star_twice_as_many() {
        let mut options = PolyStarSettings { style: PolyStarStyle::Polygon, sides: 6, star_point_size: 0.5 };
        let polygon = polystar_path(Point::new(0.0, 0.0), Point::new(10.0, 0.0), false, &options);
        // move_to + 5 line_to + close.
        assert_eq!(polygon.elements().len(), 7);
        options.style = PolyStarStyle::Star;
        let star = polystar_path(Point::new(0.0, 0.0), Point::new(10.0, 0.0), false, &options);
        assert_eq!(star.elements().len(), 13);
        let b = star.bounding_box();
        assert!((b.x1 - 10.0).abs() < 1.0e-9, "the first point is where the drag ends");
    }

    #[test]
    fn shift_snaps_the_polystar_angle() {
        let options = PolyStarSettings::default();
        let path = polystar_path(Point::new(0.0, 0.0), Point::new(10.0, 1.0), true, &options);
        let Some(anim_core::kurbo::PathEl::MoveTo(first)) = path.elements().first().copied() else { panic!("no start") };
        assert!(first.y.abs() < 1.0e-9, "snapped to zero degrees");
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
