//! Pencil and Brush: freehand strokes.

use anim_core::{Affine, BezPath, Point};
use anim_scene::Element;
use anim_shape::{EdgeShape, PathInput};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerKind, ToolEvent};
use crate::freehand::{BrushSample, brush_outline_shaped, fit_polyline, smooth_samples, thin_samples};
use crate::overlay::{Cursor, Overlay, Role};
use crate::selection::ElementRef;
use crate::settings::BrushMode;
use crate::shapeops;
use crate::tool::{Tool, ToolId, ToolOutput};

/// Distance in screen points within which the end of a pencil stroke closes it.
const CLOSE_RADIUS: f64 = 5.0;
/// Samples closer than this many screen points to the previous one are ignored while drawing.
const MIN_STEP: f64 = 1.0;

/// Draws freehand lines, smoothed or straightened.
#[derive(Debug, Clone, Default)]
pub struct PencilTool {
    points: Vec<Point>,
    active: bool,
}

impl Tool for PencilTool {
    fn id(&self) -> ToolId {
        ToolId::Pencil
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    self.points = vec![p.pos];
                    self.active = true;
                    ToolOutput::cursor(Cursor::Pencil)
                }
                PointerKind::Move => {
                    if self.active && self.points.last().is_none_or(|l| l.distance(p.pos) >= MIN_STEP / ctx.zoom.max(1.0e-6)) {
                        self.points.push(p.pos);
                    }
                    ToolOutput::cursor(Cursor::Pencil)
                }
                PointerKind::Up => {
                    if !self.active {
                        return ToolOutput::cursor(Cursor::Pencil);
                    }
                    self.active = false;
                    if self.points.last() != Some(&p.pos) {
                        self.points.push(p.pos);
                    }
                    let points = std::mem::take(&mut self.points);
                    ToolOutput::with(Cursor::Pencil, Self::commit(&points, ctx))
                }
                PointerKind::Hover => ToolOutput::cursor(Cursor::Pencil),
            },
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.cancel();
                ToolOutput::cursor(Cursor::Pencil)
            }
            ToolEvent::Cancel => {
                self.cancel();
                ToolOutput::cursor(Cursor::Pencil)
            }
            _ => ToolOutput::cursor(Cursor::Pencil),
        }
    }

    fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
        if self.points.len() < 2 {
            return Vec::new();
        }
        vec![Overlay::Polyline { points: self.points.clone(), closed: false, role: Role::Preview }]
    }

    fn cancel(&mut self) {
        self.points.clear();
        self.active = false;
    }

    fn is_busy(&self) -> bool {
        self.active
    }
}

impl PencilTool {
    fn commit(points: &[Point], ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        if points.len() < 2 || !ctx.layer_editable(ctx.layer) {
            return Vec::new();
        }
        let (Some(first), Some(last)) = (points.first(), points.last()) else { return Vec::new() };
        let closed = points.len() >= 4 && first.distance(*last) <= CLOSE_RADIUS / ctx.zoom.max(1.0e-6);
        let body: &[Point] = if closed { points.get(..points.len().saturating_sub(1)).unwrap_or(points) } else { points };
        let path = fit_polyline(body, ctx.settings.pencil_mode, ctx.zoom, closed);
        let fill = if closed { ctx.settings.active_fill() } else { None };
        let stroke = Some(ctx.settings.stroke.clone());
        let shape = EdgeShape::from_paths(&[PathInput::new(path, fill, stroke)]);
        if shape.is_empty() {
            return Vec::new();
        }
        vec![ToolCommand::DrawShape { layer: ctx.layer, frame: ctx.frame, shape, object_drawing: ctx.settings.object_drawing }]
    }
}

/// Paints filled strokes whose width follows pen pressure.
#[derive(Debug, Clone, Default)]
pub struct BrushTool {
    samples: Vec<BrushSample>,
    active: bool,
}

impl BrushTool {
    fn sample(p: &crate::event::PointerEvent, ctx: &ToolContext<'_>) -> BrushSample {
        let pressure = if ctx.settings.brush_pressure { f64::from(p.pressure).clamp(0.1, 1.0) } else { 1.0 };
        BrushSample { pos: p.pos, radius: ctx.settings.brush_size.max(0.1) / 2.0 * pressure }
    }
}

impl Tool for BrushTool {
    fn id(&self) -> ToolId {
        ToolId::Brush
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    self.samples = vec![Self::sample(p, ctx)];
                    self.active = true;
                    ToolOutput::cursor(Cursor::Brush)
                }
                PointerKind::Move => {
                    if self.active {
                        self.samples.push(Self::sample(p, ctx));
                    }
                    ToolOutput::cursor(Cursor::Brush)
                }
                PointerKind::Up => {
                    if !self.active {
                        return ToolOutput::cursor(Cursor::Brush);
                    }
                    self.active = false;
                    self.samples.push(Self::sample(p, ctx));
                    let samples = thin_samples(&std::mem::take(&mut self.samples));
                    ToolOutput::with(Cursor::Brush, Self::commit(&samples, ctx))
                }
                PointerKind::Hover => ToolOutput::cursor(Cursor::Brush),
            },
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.cancel();
                ToolOutput::cursor(Cursor::Brush)
            }
            ToolEvent::Cancel => {
                self.cancel();
                ToolOutput::cursor(Cursor::Brush)
            }
            _ => ToolOutput::cursor(Cursor::Brush),
        }
    }

    fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
        if !self.active {
            return Vec::new();
        }
        let width = self.samples.iter().map(|s| s.radius * 2.0).fold(0.0_f64, f64::max);
        let mut path = BezPath::new();
        if let Some((first, rest)) = self.samples.split_first() {
            path.move_to(first.pos);
            for s in rest {
                path.line_to(s.pos);
            }
        }
        vec![Overlay::Path { path, role: Role::Preview, width: Some(width), filled: false }]
    }

    fn cancel(&mut self) {
        self.samples.clear();
        self.active = false;
    }

    fn is_busy(&self) -> bool {
        self.active
    }
}

impl BrushTool {
    fn commit(samples: &[BrushSample], ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        if !ctx.layer_editable(ctx.layer) {
            return Vec::new();
        }
        let smoothed = smooth_samples(samples, ctx.settings.brush_smoothing);
        let Some(outline) = brush_outline_shaped(&smoothed, ctx.settings.brush_shape) else { return Vec::new() };
        let mode = ctx.settings.brush_mode;
        if mode != BrushMode::PaintNormal && !ctx.settings.object_drawing {
            let start = samples.first().map_or(Point::ZERO, |s| s.pos);
            return Self::commit_in_mode(&outline, start, mode, ctx);
        }
        let shape = EdgeShape::from_paths(&[PathInput::new(outline, Some(ctx.settings.fill.clone()), None)]);
        if shape.is_empty() {
            return Vec::new();
        }
        vec![ToolCommand::DrawShape { layer: ctx.layer, frame: ctx.frame, shape, object_drawing: ctx.settings.object_drawing }]
    }

    /// Paint fills, behind, selection and inside: the stroke is clipped against what is already on the layer.
    fn commit_in_mode(outline: &BezPath, start: Point, mode: BrushMode, ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        let fill = &ctx.settings.fill;
        if mode == BrushMode::PaintSelection {
            let mut commands = Vec::new();
            for r in &ctx.selection.items {
                let Some(resolved) = ctx.resolve(r) else { continue };
                let Element::Shape(s) = resolved.element else { continue };
                let to_stage = resolved.local_to_stage();
                if to_stage.determinant().abs() < 1.0e-12 {
                    continue;
                }
                let local = to_stage.inverse() * outline.clone();
                if let Some(shape) = shapeops::paint_clipped(&s.shape, &local, fill, mode, to_stage.inverse() * start) {
                    commands.push(ToolCommand::ReplaceShape { target: r.clone(), shape });
                }
            }
            return commands;
        }
        let Some(keyframe) = ctx.layer_ref(ctx.layer).and_then(|l| l.keyframe_at(ctx.frame)) else { return Self::fallback(outline, ctx) };
        let found = keyframe.elements.iter().enumerate().find_map(|(i, e)| match e {
            Element::Shape(s) if !s.is_drawing_object && s.matrix == Affine::IDENTITY => Some((i, s)),
            _ => None,
        });
        let Some((index, existing)) = found else { return Self::fallback(outline, ctx) };
        shapeops::paint_clipped(&existing.shape, outline, fill, mode, start)
            .map(|shape| vec![ToolCommand::ReplaceShape { target: ElementRef::top_level(ctx.layer, keyframe.index, index), shape }])
            .unwrap_or_default()
    }

    /// Nothing is on the layer: behind and fills paint like normal, inside and selection paint nothing.
    fn fallback(outline: &BezPath, ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        if matches!(ctx.settings.brush_mode, BrushMode::PaintInside | BrushMode::PaintSelection) {
            return Vec::new();
        }
        let shape = EdgeShape::from_paths(&[PathInput::new(outline.clone(), Some(ctx.settings.fill.clone()), None)]);
        if shape.is_empty() {
            return Vec::new();
        }
        vec![ToolCommand::DrawShape { layer: ctx.layer, frame: ctx.frame, shape, object_drawing: false }]
    }
}
