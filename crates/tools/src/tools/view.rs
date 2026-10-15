//! Hand, Zoom and Eyedropper: tools that do not draw.

use anim_core::{Point, Rect, Vec2};
use anim_scene::Element;

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerKind, ToolEvent};
use crate::hit::{PickPart, pick};
use crate::overlay::{Cursor, Overlay};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Zoom factor of one click.
const ZOOM_STEP: f64 = 2.0;

/// Pans the stage by dragging.
#[derive(Debug, Clone, Default)]
pub struct HandTool {
    last: Option<Point>,
}

impl Tool for HandTool {
    fn id(&self) -> ToolId {
        ToolId::Hand
    }

    fn handle(&mut self, event: &ToolEvent, _ctx: &ToolContext<'_>) -> ToolOutput {
        let ToolEvent::Pointer(p) = event else { return ToolOutput::cursor(Cursor::Hand) };
        match p.kind {
            PointerKind::Down => {
                self.last = Some(p.screen_pos());
                ToolOutput::cursor(Cursor::Grabbing)
            }
            PointerKind::Move => {
                let now = p.screen_pos();
                let Some(last) = self.last.replace(now) else { return ToolOutput::cursor(Cursor::Grabbing) };
                let delta = now - last;
                if delta == Vec2::ZERO {
                    return ToolOutput::cursor(Cursor::Grabbing);
                }
                ToolOutput::with(Cursor::Grabbing, vec![ToolCommand::PanScreen { delta }])
            }
            PointerKind::Up => {
                self.last = None;
                ToolOutput::cursor(Cursor::Hand)
            }
            PointerKind::Hover => ToolOutput::cursor(Cursor::Hand),
        }
    }

    fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
        Vec::new()
    }

    fn cancel(&mut self) {
        self.last = None;
    }

    fn is_busy(&self) -> bool {
        self.last.is_some()
    }
}

/// Zooms by clicking or dragging a rectangle.
#[derive(Debug, Clone, Default)]
pub struct ZoomTool {
    start: Option<Point>,
    current: Point,
}

impl Tool for ZoomTool {
    fn id(&self) -> ToolId {
        ToolId::Zoom
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => {
                let cursor = if p.modifiers.alt { Cursor::ZoomOut } else { Cursor::ZoomIn };
                match p.kind {
                    PointerKind::Down => {
                        self.start = Some(p.pos);
                        self.current = p.pos;
                        ToolOutput::cursor(cursor)
                    }
                    PointerKind::Move => {
                        self.current = p.pos;
                        ToolOutput::cursor(cursor)
                    }
                    PointerKind::Up => {
                        let Some(start) = self.start.take() else { return ToolOutput::cursor(cursor) };
                        let rect = Rect::from_points(start, p.pos);
                        let drag = ctx.drag_threshold();
                        if !p.modifiers.alt && rect.width() > drag && rect.height() > drag {
                            ToolOutput::with(cursor, vec![ToolCommand::ZoomToRect { rect }])
                        } else {
                            let factor = if p.modifiers.alt { 1.0 / ZOOM_STEP } else { ZOOM_STEP };
                            ToolOutput::with(cursor, vec![ToolCommand::ZoomAt { center: p.pos, factor }])
                        }
                    }
                    PointerKind::Hover => ToolOutput::cursor(cursor),
                }
            }
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.start = None;
                ToolOutput::cursor(Cursor::ZoomIn)
            }
            ToolEvent::Cancel => {
                self.start = None;
                ToolOutput::cursor(Cursor::ZoomIn)
            }
            _ => ToolOutput::cursor(Cursor::ZoomIn),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        match self.start {
            Some(s) if Rect::from_points(s, self.current).width() > ctx.drag_threshold() => vec![Overlay::Marquee { rect: Rect::from_points(s, self.current) }],
            _ => Vec::new(),
        }
    }

    fn cancel(&mut self) {
        self.start = None;
    }

    fn is_busy(&self) -> bool {
        self.start.is_some()
    }
}

/// Copies the style under the pointer into the toolbox.
#[derive(Debug, Clone, Default)]
pub struct EyedropperTool;

impl Tool for EyedropperTool {
    fn id(&self) -> ToolId {
        ToolId::Eyedropper
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let ToolEvent::Pointer(p) = event else { return ToolOutput::cursor(Cursor::Eyedropper) };
        if p.kind != PointerKind::Down {
            return ToolOutput::cursor(Cursor::Eyedropper);
        }
        let Some(hit) = pick(ctx, p.pos) else { return ToolOutput::cursor(Cursor::Eyedropper) };
        let Some(resolved) = ctx.resolve(&hit.element) else { return ToolOutput::cursor(Cursor::Eyedropper) };
        let Element::Shape(shape) = resolved.element else { return ToolOutput::cursor(Cursor::Eyedropper) };
        let commands = match hit.part {
            PickPart::Fill { style } => shape.shape.fill(style).map_or_else(Vec::new, |fill| {
                vec![ToolCommand::SetToolStyle { fill: Some(fill.clone()), stroke: None }, ToolCommand::ActivateTool { tool: ToolId::PaintBucket }]
            }),
            PickPart::Stroke { style, .. } => shape.shape.stroke(style).map_or_else(Vec::new, |stroke| {
                vec![ToolCommand::SetToolStyle { fill: None, stroke: Some(stroke.clone()) }, ToolCommand::ActivateTool { tool: ToolId::InkBottle }]
            }),
            PickPart::Body => Vec::new(),
        };
        ToolOutput::with(Cursor::Eyedropper, commands)
    }

    fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
        Vec::new()
    }

    fn cancel(&mut self) {}

    fn is_busy(&self) -> bool {
        false
    }
}
