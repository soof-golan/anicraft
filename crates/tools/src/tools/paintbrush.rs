//! The Paint Brush tool: art brushes (an outline whose width follows a rule) and pattern brushes (a tile repeated
//! along the path), painted with the stroke colour.

use std::f64::consts::{FRAC_PI_4, PI, TAU};

use anim_core::kurbo::{Circle, Ellipse, Shape as _};
use anim_core::num::usize_to_f64;
use anim_core::{Affine, BezPath, Point, Vec2};
use anim_shape::{EdgeShape, PathInput};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerKind, ToolEvent};
use crate::freehand::{BrushSample, brush_outline, fit_polyline};
use crate::overlay::{Cursor, Overlay, Role};
use crate::settings::{PaintBrushSettings, PaintBrushStyle, PencilMode};
use crate::tool::{Tool, ToolId, ToolOutput};
use crate::widthline::{PathSample, sample_path};

/// Samples closer than this many screen points to the previous one are ignored while drawing.
const MIN_STEP: f64 = 1.0;
/// Most tiles one stroke may repeat.
const MAX_TILES: usize = 4000;

/// A unit tile centred on the origin, pointing along +x, about one pixel across.
fn tile(style: PaintBrushStyle) -> BezPath {
    match style {
        PaintBrushStyle::Dots => Circle::new((0.0, 0.0), 0.5).to_path(0.01),
        PaintBrushStyle::Stars => {
            let mut p = BezPath::new();
            for i in 0..10_u32 {
                let r = if i % 2 == 0 { 0.5 } else { 0.22 };
                let a = f64::from(i) * PI / 5.0;
                let pt = Point::new(r * a.cos(), r * a.sin());
                if i == 0 {
                    p.move_to(pt);
                } else {
                    p.line_to(pt);
                }
            }
            p.close_path();
            p
        }
        PaintBrushStyle::Arrows => {
            let mut p = BezPath::new();
            p.move_to((0.5, 0.0));
            p.line_to((-0.4, 0.42));
            p.line_to((-0.2, 0.0));
            p.line_to((-0.4, -0.42));
            p.close_path();
            p
        }
        PaintBrushStyle::Dashes => {
            let mut p = BezPath::new();
            p.move_to((-0.5, -0.17));
            p.line_to((0.5, -0.17));
            p.line_to((0.5, 0.17));
            p.line_to((-0.5, 0.17));
            p.close_path();
            p
        }
        PaintBrushStyle::Chain => {
            let mut p = Ellipse::new((0.0, 0.0), (0.5, 0.3), 0.0).to_path(0.01);
            // The hole runs the other way so the link is a ring.
            let mut hole = Ellipse::new((0.0, 0.0), (0.33, 0.13), 0.0).to_path(0.01);
            hole = reverse(&hole);
            p.extend(hole.iter());
            p
        }
        _ => BezPath::new(),
    }
}

/// The same outline traversed the other way (polygon approximation: tiles are small).
fn reverse(path: &BezPath) -> BezPath {
    let mut points: Vec<Point> = Vec::new();
    anim_core::kurbo::flatten(path.iter(), 0.01, |el| match el {
        anim_core::kurbo::PathEl::MoveTo(p) | anim_core::kurbo::PathEl::LineTo(p) => points.push(p),
        _ => {}
    });
    points.reverse();
    let mut out = BezPath::new();
    for (i, p) in points.iter().enumerate() {
        if i == 0 {
            out.move_to(*p);
        } else {
            out.line_to(*p);
        }
    }
    out.close_path();
    out
}

/// A cheap deterministic grain in 0..1 for the charcoal brush.
fn grain(i: usize) -> f64 {
    let x = usize_to_f64(i).mul_add(12.9898, 78.233).sin() * 43758.5453;
    x - x.floor()
}

fn art_radius(style: PaintBrushStyle, sample: &PathSample, index: usize, size: f64) -> f64 {
    let half = size / 2.0;
    match style {
        PaintBrushStyle::Calligraphic => {
            let angle = sample.tangent.y.atan2(sample.tangent.x);
            half * 0.85_f64.mul_add((angle - FRAC_PI_4).sin().abs(), 0.15)
        }
        PaintBrushStyle::Tapered => half * (PI * sample.t).sin().powf(0.6).max(0.04),
        PaintBrushStyle::Charcoal => half * 0.45_f64.mul_add(grain(index), 0.55),
        _ => half,
    }
}

/// The filled outline of a stroke along `path` with the brush, or `None` when there is nothing to paint.
#[must_use]
pub fn paint_brush_outline(path: &BezPath, settings: &PaintBrushSettings) -> Option<BezPath> {
    let size = settings.size.max(0.5);
    if settings.style.is_pattern() {
        let step = (size * settings.spacing / 100.0).max(1.0);
        let samples = sample_path(path, step);
        let mut out = BezPath::new();
        let tile = tile(settings.style);
        for sample in samples.iter().take(MAX_TILES) {
            let angle = if sample.tangent == Vec2::ZERO { 0.0 } else { sample.tangent.y.atan2(sample.tangent.x) };
            let matrix = Affine::translate(sample.pos.to_vec2()) * Affine::rotate(angle % TAU) * Affine::scale(size);
            out.extend((matrix * tile.clone()).iter());
        }
        return (!out.elements().is_empty()).then_some(out);
    }
    let samples: Vec<BrushSample> =
        sample_path(path, 1.0).iter().enumerate().map(|(i, s)| BrushSample { pos: s.pos, radius: art_radius(settings.style, s, i, size).max(0.05) }).collect();
    brush_outline(&samples)
}

/// Paints with an art or pattern brush.
#[derive(Debug, Clone, Default)]
pub struct PaintBrushTool {
    points: Vec<Point>,
    active: bool,
}

impl PaintBrushTool {
    fn commit(points: &[Point], ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        if points.len() < 2 || !ctx.layer_editable(ctx.layer) {
            return Vec::new();
        }
        let s = ctx.settings;
        let mode = if s.paint_brush.smoothing < 20.0 { PencilMode::Ink } else { PencilMode::Smooth };
        let path = fit_polyline(points, mode, ctx.zoom, false);
        let Some(outline) = paint_brush_outline(&path, &s.paint_brush) else { return Vec::new() };
        // The paint brush paints with the stroke colour, like a pen.
        let paint = if s.stroke_enabled { s.stroke.solid().fill.clone() } else { s.fill.clone() };
        let shape = EdgeShape::from_paths(&[PathInput::new(outline, Some(paint), None)]);
        if shape.is_empty() {
            return Vec::new();
        }
        vec![ToolCommand::DrawShape { layer: ctx.layer, frame: ctx.frame, shape, object_drawing: s.object_drawing }]
    }
}

impl Tool for PaintBrushTool {
    fn id(&self) -> ToolId {
        ToolId::PaintBrush
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    self.points = vec![p.pos];
                    self.active = true;
                    ToolOutput::cursor(Cursor::Brush)
                }
                PointerKind::Move => {
                    if self.active && self.points.last().is_none_or(|l| l.distance(p.pos) >= MIN_STEP / ctx.zoom.max(1.0e-6)) {
                        self.points.push(p.pos);
                    }
                    ToolOutput::cursor(Cursor::Brush)
                }
                PointerKind::Up => {
                    if !self.active {
                        return ToolOutput::cursor(Cursor::Brush);
                    }
                    self.active = false;
                    if self.points.last() != Some(&p.pos) {
                        self.points.push(p.pos);
                    }
                    let points = std::mem::take(&mut self.points);
                    ToolOutput::with(Cursor::Brush, Self::commit(&points, ctx))
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

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        if self.points.len() < 2 {
            return Vec::new();
        }
        let mut path = BezPath::new();
        for (i, p) in self.points.iter().enumerate() {
            if i == 0 {
                path.move_to(*p);
            } else {
                path.line_to(*p);
            }
        }
        paint_brush_outline(&path, &ctx.settings.paint_brush).map_or_else(
            || vec![Overlay::Polyline { points: self.points.clone(), closed: false, role: Role::Preview }],
            |outline| vec![Overlay::Path { path: outline, role: Role::Preview, width: None, filled: true }],
        )
    }

    fn cancel(&mut self) {
        self.points.clear();
        self.active = false;
    }

    fn is_busy(&self) -> bool {
        self.active
    }
}
