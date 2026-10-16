//! The Gradient Transform tool: move, rotate and scale a gradient or bitmap fill, and move a radial focal point.

use anim_core::{Affine, Point};
use anim_scene::Element;
use anim_shape::{BITMAP_MATRIX_SCALE, EdgeShape, FillStyle};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerKind, ToolEvent};
use crate::fillfit::GRADIENT_HALF;
use crate::hit::{PickPart, pick};
use crate::overlay::{Cursor, Marker, Overlay, Role};
use crate::selection::ElementRef;
use crate::tool::{Tool, ToolId, ToolOutput};

/// Handle hit radius in screen points.
const HANDLE_RADIUS: f64 = 7.0;
/// Size of a bitmap without a known size, in pixels.
const FALLBACK_BITMAP: f64 = 100.0;
/// Smallest scale a drag may reach, so a fill never collapses.
const MIN_SCALE: f64 = 0.01;
/// Rotation snap with shift, in radians.
const ROTATE_SNAP: f64 = std::f64::consts::PI / 12.0;

/// What a handle does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grip {
    /// The centre: moves the fill.
    Move,
    /// The square: stretches the fill along its x axis.
    Width,
    /// The second square of a bitmap: stretches the fill along its y axis.
    Height,
    /// The circle on the edge of a radial gradient: scales it uniformly.
    Size,
    /// The rotation handle.
    Rotate,
    /// The triangle of a radial gradient: moves the focal point along the x axis.
    Focal,
}

/// A fill being edited: where its handles are.
#[derive(Debug, Clone)]
struct Frame {
    /// Maps fill space (gradient space or image pixels) to the stage.
    to_stage: Affine,
    /// Maps shape space to the stage.
    shape_to_stage: Affine,
    centre: Point,
    handles: Vec<(Grip, Point)>,
    /// Outline of the fill's box in fill space.
    outline: [Point; 4],
    /// Half the length of the x axis in fill space, which the focal ratio is relative to.
    half_x: f64,
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    grip: Grip,
    start: Point,
    current: Point,
    snap: bool,
}

/// Edits the placement of gradient and bitmap fills.
#[derive(Debug, Clone, Default)]
pub struct GradientTransformTool {
    /// The shape and the 1-based fill style being edited.
    target: Option<(ElementRef, u32)>,
    drag: Option<Drag>,
}

fn frame_of(ctx: &ToolContext<'_>, target: &(ElementRef, u32)) -> Option<Frame> {
    let resolved = ctx.resolve(&target.0)?;
    let Element::Shape(s) = resolved.element else { return None };
    let shape_to_stage = resolved.local_to_stage();
    let det = shape_to_stage.determinant();
    if !det.is_finite() || det.abs() < 1.0e-12 {
        return None;
    }
    let h = GRADIENT_HALF;
    match s.shape.fill(target.1)? {
        FillStyle::LinearGradient(g) => Some(Frame {
            to_stage: shape_to_stage * g.matrix,
            shape_to_stage,
            centre: Point::ZERO,
            handles: vec![(Grip::Move, Point::ZERO), (Grip::Width, Point::new(h, 0.0)), (Grip::Rotate, Point::new(0.0, -h))],
            outline: [Point::new(-h, -h), Point::new(h, -h), Point::new(h, h), Point::new(-h, h)],
            half_x: h,
        }),
        FillStyle::RadialGradient(g) => Some(Frame {
            to_stage: shape_to_stage * g.matrix,
            shape_to_stage,
            centre: Point::ZERO,
            handles: vec![
                (Grip::Move, Point::ZERO),
                (Grip::Focal, Point::new(g.focal.clamp(-1.0, 1.0) * h, 0.0)),
                (Grip::Width, Point::new(h, 0.0)),
                (Grip::Size, Point::new(0.0, h)),
                (Grip::Rotate, Point::new(0.0, -h)),
            ],
            outline: [Point::new(-h, -h), Point::new(h, -h), Point::new(h, h), Point::new(-h, h)],
            half_x: h,
        }),
        FillStyle::Bitmap(b) => {
            let [w, ht] = ctx.document.library.find_bitmap(&b.path).and_then(|i| i.size).unwrap_or([FALLBACK_BITMAP, FALLBACK_BITMAP]);
            Some(Frame {
                to_stage: shape_to_stage * b.matrix * Affine::scale(BITMAP_MATRIX_SCALE),
                shape_to_stage,
                centre: Point::new(w / 2.0, ht / 2.0),
                handles: vec![
                    (Grip::Move, Point::new(w / 2.0, ht / 2.0)),
                    (Grip::Width, Point::new(w, ht / 2.0)),
                    (Grip::Height, Point::new(w / 2.0, ht)),
                    (Grip::Rotate, Point::new(w / 2.0, 0.0)),
                ],
                outline: [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, ht), Point::new(0.0, ht)],
                half_x: w / 2.0,
            })
        }
        FillStyle::Solid(_) => None,
    }
}

fn about(centre: Point, m: Affine) -> Affine {
    Affine::translate(centre.to_vec2()) * m * Affine::translate(-centre.to_vec2())
}

/// The stage-space change that drags `grip` from its resting place `handle` to `current`; `None` for the focal
/// grip, which is not a transform.
fn delta(frame: &Frame, grip: Grip, handle: Point, drag: &Drag) -> Option<Affine> {
    let centre = frame.to_stage * frame.centre;
    let from = handle - centre;
    let to = drag.current - centre;
    match grip {
        Grip::Move => Some(Affine::translate(drag.current - drag.start)),
        Grip::Width | Grip::Height => {
            let len = from.hypot();
            if len < 1.0e-9 {
                return None;
            }
            let axis = from / len;
            let s = (to.dot(axis) / len).abs().max(MIN_SCALE) * if to.dot(axis) < 0.0 { -1.0 } else { 1.0 };
            let r = Affine::rotate(axis.y.atan2(axis.x));
            Some(about(centre, r * Affine::scale_non_uniform(s, 1.0) * r.inverse()))
        }
        Grip::Size => {
            let len = from.hypot();
            if len < 1.0e-9 {
                return None;
            }
            Some(about(centre, Affine::scale((to.hypot() / len).max(MIN_SCALE))))
        }
        Grip::Rotate => {
            let mut angle = to.y.atan2(to.x) - from.y.atan2(from.x);
            if drag.snap {
                angle = (angle / ROTATE_SNAP).round() * ROTATE_SNAP;
            }
            Some(about(centre, Affine::rotate(angle)))
        }
        Grip::Focal => None,
    }
}

fn finite(m: Affine) -> bool {
    m.as_coeffs().iter().all(|c| c.is_finite())
}

/// The shape with the fill `style` changed by `delta` (stage space), or its focal point moved.
fn edited_shape(ctx: &ToolContext<'_>, target: &(ElementRef, u32), frame: &Frame, drag: &Drag) -> Option<EdgeShape> {
    let resolved = ctx.resolve(&target.0)?;
    let Element::Shape(s) = resolved.element else { return None };
    let mut shape = s.shape.clone();
    let index = usize::try_from(target.1).ok()?.checked_sub(1)?;
    let handle_local = frame.handles.iter().find(|(g, _)| *g == drag.grip)?.1;
    let handle = frame.to_stage * handle_local;
    let fill = shape.fills.get_mut(index)?;
    if drag.grip == Grip::Focal {
        let centre = frame.to_stage * frame.centre;
        let axis = frame.to_stage * Point::new(frame.half_x, 0.0) - centre;
        let len = axis.hypot();
        if len < 1.0e-9 {
            return None;
        }
        let focal = ((drag.current - centre).dot(axis / len) / len).clamp(-0.99, 0.99);
        if let FillStyle::RadialGradient(g) = fill {
            g.focal = focal;
        }
        return Some(shape);
    }
    let d = delta(frame, drag.grip, handle, drag)?;
    // Conjugate into shape space: S^-1 D S.
    let change = frame.shape_to_stage.inverse() * d * frame.shape_to_stage;
    if !finite(change) {
        return None;
    }
    match fill {
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => g.matrix = change * g.matrix,
        FillStyle::Bitmap(b) => b.matrix = change * b.matrix,
        FillStyle::Solid(_) => return None,
    }
    finite(match fill {
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => g.matrix,
        FillStyle::Bitmap(b) => b.matrix,
        FillStyle::Solid(_) => Affine::IDENTITY,
    })
    .then_some(shape)
}

impl GradientTransformTool {
    fn grip_at(&self, ctx: &ToolContext<'_>, p: Point) -> Option<Grip> {
        let target = self.target.as_ref()?;
        let frame = frame_of(ctx, target)?;
        let reach = HANDLE_RADIUS / ctx.zoom.max(1.0e-6);
        frame
            .handles
            .iter()
            .map(|(g, local)| (*g, (frame.to_stage * *local).distance(p)))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g)
    }

    /// Picks a gradient or bitmap fill to edit.
    fn pick_fill(&mut self, ctx: &ToolContext<'_>, p: Point) {
        self.target = pick(ctx, p).and_then(|hit| {
            let PickPart::Fill { style } = hit.part else { return None };
            let resolved = ctx.resolve(&hit.element)?;
            let Element::Shape(s) = resolved.element else { return None };
            (!matches!(s.shape.fill(style)?, FillStyle::Solid(_))).then_some((hit.element, style))
        });
    }
}

const fn cursor_for(grip: Option<Grip>) -> Cursor {
    match grip {
        Some(Grip::Move) => Cursor::Move,
        Some(Grip::Rotate) => Cursor::Rotate,
        Some(Grip::Width | Grip::Height | Grip::Size) => Cursor::Resize(crate::overlay::Compass::E),
        Some(Grip::Focal) => Cursor::Point,
        None => Cursor::Default,
    }
}

impl Tool for GradientTransformTool {
    fn id(&self) -> ToolId {
        ToolId::GradientTransform
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    if let Some(grip) = self.grip_at(ctx, p.pos) {
                        self.drag = Some(Drag { grip, start: p.pos, current: p.pos, snap: p.modifiers.shift });
                        return ToolOutput::cursor(cursor_for(Some(grip)));
                    }
                    self.pick_fill(ctx, p.pos);
                    ToolOutput::none()
                }
                PointerKind::Move => {
                    if let Some(d) = &mut self.drag {
                        d.current = p.pos;
                        d.snap = p.modifiers.shift;
                        return ToolOutput::cursor(cursor_for(Some(d.grip)));
                    }
                    ToolOutput::none()
                }
                PointerKind::Up => {
                    let Some(mut drag) = self.drag.take() else { return ToolOutput::none() };
                    drag.current = p.pos;
                    drag.snap = p.modifiers.shift;
                    let Some(target) = self.target.clone() else { return ToolOutput::none() };
                    let Some(frame) = frame_of(ctx, &target) else { return ToolOutput::none() };
                    match edited_shape(ctx, &target, &frame, &drag) {
                        Some(shape) => ToolOutput::with(cursor_for(Some(drag.grip)), vec![ToolCommand::ReplaceShape { target: target.0, shape }]),
                        None => ToolOutput::none(),
                    }
                }
                PointerKind::Hover => ToolOutput::cursor(cursor_for(self.grip_at(ctx, p.pos))),
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
        let Some(target) = &self.target else { return Vec::new() };
        let Some(frame) = frame_of(ctx, target) else { return Vec::new() };
        // While dragging, show the edited placement.
        let shown = self.drag.as_ref().and_then(|d| {
            let shape = edited_shape(ctx, target, &frame, d)?;
            let index = usize::try_from(target.1).ok()?.checked_sub(1)?;
            fill_to_stage(shape.fills.get(index)?, &frame)
        });
        let to_stage = shown.unwrap_or(frame.to_stage);
        let focal_at = self.drag.filter(|d| d.grip == Grip::Focal).map(|d| d.current);
        let mut out = Vec::new();
        let corners: Vec<Point> = frame.outline.iter().map(|c| to_stage * *c).collect();
        out.push(Overlay::Polyline { points: corners, closed: true, role: Role::Guide });
        let centre = to_stage * frame.centre;
        for (grip, local) in &frame.handles {
            let pos = if *grip == Grip::Focal { focal_at.unwrap_or_else(|| frame.to_stage * *local) } else { to_stage * *local };
            if *grip != Grip::Move {
                out.push(Overlay::Line { a: centre, b: pos, role: Role::Guide });
            }
            let (marker, filled) = match grip {
                Grip::Move | Grip::Size => (Marker::Circle, true),
                Grip::Width | Grip::Height => (Marker::Square, false),
                Grip::Rotate => (Marker::Circle, false),
                Grip::Focal => (Marker::Diamond, true),
            };
            out.push(Overlay::Marker { pos, marker, role: Role::Handle, filled });
        }
        out
    }

    fn cancel(&mut self) {
        self.drag = None;
    }

    fn is_busy(&self) -> bool {
        self.drag.is_some()
    }

    fn activated(&mut self, ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        // Start on the selected shape's first gradient or bitmap fill.
        if self.target.is_none()
            && let [only] = ctx.selection.items.as_slice()
            && let Some(resolved) = ctx.resolve(only)
            && let Element::Shape(s) = resolved.element
        {
            self.target = s
                .shape
                .fills
                .iter()
                .position(|f| !matches!(f, FillStyle::Solid(_)))
                .and_then(|i| u32::try_from(i.saturating_add(1)).ok())
                .map(|style| (only.clone(), style));
        }
        Vec::new()
    }
}

/// The fill-to-stage matrix of an edited fill.
fn fill_to_stage(fill: &FillStyle, frame: &Frame) -> Option<Affine> {
    match fill {
        FillStyle::LinearGradient(g) | FillStyle::RadialGradient(g) => Some(frame.shape_to_stage * g.matrix),
        FillStyle::Bitmap(b) => Some(frame.shape_to_stage * b.matrix * Affine::scale(BITMAP_MATRIX_SCALE)),
        FillStyle::Solid(_) => None,
    }
}
