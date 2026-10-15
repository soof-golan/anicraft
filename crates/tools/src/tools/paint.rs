//! Paint Bucket, Ink Bottle and Eraser: tools that restyle or cut existing shapes.

use anim_core::{Affine, Point};
use anim_scene::Element;
use anim_shape::EdgeShape;

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerKind, ToolEvent};
use crate::hit::{Candidate, candidates, scale_of};
use crate::overlay::{Cursor, Overlay, Role};
use crate::settings::EraserMode;
use crate::shapeops::{self, EraseSpec, Erased};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Shapes with more segments than this are not analysed for enclosed areas.
const MAX_SEGMENTS: usize = 20_000;

fn segment_count(shape: &EdgeShape) -> usize {
    shape.edges.iter().map(|e| e.segments.len()).sum()
}

/// A shape candidate with the matrix that maps stage coordinates into the shape's own pixels.
struct ShapeTarget<'a> {
    candidate: Candidate<'a>,
    shape: &'a EdgeShape,
    to_local: Affine,
}

/// The shape elements that can be edited, topmost first (the current layer's before the others').
fn shape_targets<'a>(ctx: &ToolContext<'a>) -> Vec<ShapeTarget<'a>> {
    let mut targets: Vec<ShapeTarget<'a>> = candidates(ctx)
        .into_iter()
        .filter_map(|candidate| {
            let Element::Shape(s) = candidate.element else { return None };
            let to_stage = candidate.parent_to_stage * s.matrix;
            let det = to_stage.determinant();
            if !det.is_finite() || det.abs() < 1.0e-12 {
                return None;
            }
            Some(ShapeTarget { shape: &s.shape, to_local: to_stage.inverse(), candidate })
        })
        .collect();
    targets.sort_by_key(|t| t.candidate.element_ref.layer != ctx.layer);
    targets
}

/// Fills the area under the pointer.
#[derive(Debug, Clone, Default)]
pub struct PaintBucketTool;

impl Tool for PaintBucketTool {
    fn id(&self) -> ToolId {
        ToolId::PaintBucket
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let ToolEvent::Pointer(p) = event else { return ToolOutput::cursor(Cursor::Bucket) };
        if p.kind != PointerKind::Down {
            return ToolOutput::cursor(Cursor::Bucket);
        }
        for target in shape_targets(ctx) {
            if segment_count(target.shape) > MAX_SEGMENTS {
                continue;
            }
            let local = target.to_local * p.pos;
            let gap = ctx.settings.bucket_gap.pixels() * scale_of(target.to_local);
            if let Some(shape) = shapeops::paint_bucket(target.shape, local, &ctx.settings.fill, gap) {
                return ToolOutput::with(Cursor::Bucket, vec![ToolCommand::ReplaceShape { target: target.candidate.element_ref, shape }]);
            }
        }
        ToolOutput::cursor(Cursor::Bucket)
    }

    fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
        Vec::new()
    }

    fn cancel(&mut self) {}

    fn is_busy(&self) -> bool {
        false
    }
}

/// Restyles strokes and outlines fills.
#[derive(Debug, Clone, Default)]
pub struct InkBottleTool;

impl Tool for InkBottleTool {
    fn id(&self) -> ToolId {
        ToolId::InkBottle
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let ToolEvent::Pointer(p) = event else { return ToolOutput::cursor(Cursor::InkBottle) };
        if p.kind != PointerKind::Down {
            return ToolOutput::cursor(Cursor::InkBottle);
        }
        for target in shape_targets(ctx) {
            if segment_count(target.shape) > MAX_SEGMENTS {
                continue;
            }
            let local = target.to_local * p.pos;
            let tolerance = ctx.tolerance() * scale_of(target.to_local);
            if let Some(shape) = shapeops::ink_bottle(target.shape, local, tolerance, &ctx.settings.stroke) {
                return ToolOutput::with(Cursor::InkBottle, vec![ToolCommand::ReplaceShape { target: target.candidate.element_ref, shape }]);
            }
        }
        ToolOutput::cursor(Cursor::InkBottle)
    }

    fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
        Vec::new()
    }

    fn cancel(&mut self) {}

    fn is_busy(&self) -> bool {
        false
    }
}

/// Erases strokes and fills along a dragged path.
#[derive(Debug, Clone, Default)]
pub struct EraserTool {
    points: Vec<Point>,
    active: bool,
    hover: Option<Point>,
}

impl EraserTool {
    fn commit(&self, ctx: &ToolContext<'_>) -> Vec<ToolCommand> {
        let mode = ctx.settings.eraser_mode;
        let radius = ctx.settings.eraser_size.max(0.1) / 2.0;
        let (fills, lines) = match mode {
            EraserMode::Normal | EraserMode::SelectedFills => (true, true),
            EraserMode::Fills | EraserMode::Inside => (true, false),
            EraserMode::Lines => (false, true),
        };
        let mut replaced = Vec::new();
        let mut gone = Vec::new();
        for target in shape_targets(ctx) {
            if mode == EraserMode::SelectedFills && !ctx.selection.contains(&target.candidate.element_ref) {
                continue;
            }
            let local: Vec<Point> = self.points.iter().map(|&p| target.to_local * p).collect();
            let only_fill_at = (mode == EraserMode::Inside).then(|| local.first().copied()).flatten();
            let spec = EraseSpec { points: &local, radius: radius * scale_of(target.to_local), fills, lines, only_fill_at };
            match shapeops::erase(target.shape, &spec) {
                Erased::Unchanged => {}
                Erased::Changed(shape) => replaced.push(ToolCommand::ReplaceShape { target: target.candidate.element_ref, shape }),
                Erased::Gone => gone.push(target.candidate.element_ref),
            }
        }
        if !gone.is_empty() {
            replaced.push(ToolCommand::DeleteElements { targets: gone });
        }
        replaced
    }
}

impl Tool for EraserTool {
    fn id(&self) -> ToolId {
        ToolId::Eraser
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    self.points = vec![p.pos];
                    self.active = true;
                    ToolOutput::cursor(Cursor::Eraser)
                }
                PointerKind::Move => {
                    self.hover = Some(p.pos);
                    if self.active && self.points.last().is_none_or(|l| l.distance(p.pos) >= 0.5 / ctx.zoom.max(1.0e-6)) {
                        self.points.push(p.pos);
                    }
                    ToolOutput::cursor(Cursor::Eraser)
                }
                PointerKind::Up => {
                    if !self.active {
                        return ToolOutput::cursor(Cursor::Eraser);
                    }
                    self.active = false;
                    if self.points.last() != Some(&p.pos) {
                        self.points.push(p.pos);
                    }
                    let commands = self.commit(ctx);
                    self.points.clear();
                    ToolOutput::with(Cursor::Eraser, commands)
                }
                PointerKind::Hover => {
                    self.hover = Some(p.pos);
                    ToolOutput::cursor(Cursor::Eraser)
                }
            },
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.cancel();
                ToolOutput::cursor(Cursor::Eraser)
            }
            ToolEvent::Cancel => {
                self.cancel();
                ToolOutput::cursor(Cursor::Eraser)
            }
            _ => ToolOutput::cursor(Cursor::Eraser),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let width = ctx.settings.eraser_size.max(0.1);
        let mut out = Vec::new();
        if self.active && self.points.len() >= 2 {
            let mut path = anim_core::BezPath::new();
            if let Some((first, rest)) = self.points.split_first() {
                path.move_to(*first);
                for p in rest {
                    path.line_to(*p);
                }
            }
            out.push(Overlay::Path { path, role: Role::Preview, width: Some(width), filled: false });
        }
        if let Some(h) = self.hover {
            out.push(Overlay::Marker { pos: h, marker: crate::overlay::Marker::Circle, role: Role::Hover, filled: false });
        }
        out
    }

    fn cancel(&mut self) {
        self.points.clear();
        self.active = false;
    }

    fn is_busy(&self) -> bool {
        self.active
    }
}
