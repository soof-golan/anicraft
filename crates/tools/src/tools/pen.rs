//! The Pen tool: bezier paths from clicks and drags.

use anim_core::{BezPath, Point};
use anim_shape::{EdgeShape, PathInput};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerKind, ToolEvent};
use crate::overlay::{Cursor, Node, Overlay, Role};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Distance in screen points within which a click on the first anchor closes the path.
const CLOSE_RADIUS: f64 = 6.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Anchor {
    pos: Point,
    handle_in: Point,
    handle_out: Point,
}

impl Anchor {
    const fn corner(pos: Point) -> Self {
        Self { pos, handle_in: pos, handle_out: pos }
    }

    fn is_corner(&self) -> bool {
        self.handle_in == self.pos && self.handle_out == self.pos
    }
}

/// Builds the path through `anchors`, closed when asked.
fn build_path(anchors: &[Anchor], closed: bool) -> BezPath {
    let mut path = BezPath::new();
    let Some(first) = anchors.first() else { return path };
    path.move_to(first.pos);
    let mut previous = *first;
    let join = |path: &mut BezPath, a: &Anchor, b: &Anchor| {
        if a.handle_out == a.pos && b.handle_in == b.pos {
            path.line_to(b.pos);
        } else {
            path.curve_to(a.handle_out, b.handle_in, b.pos);
        }
    };
    for next in anchors.iter().skip(1) {
        join(&mut path, &previous, next);
        previous = *next;
    }
    if closed {
        join(&mut path, &previous, first);
        path.close_path();
    }
    path
}

/// Draws bezier paths.
#[derive(Debug, Clone, Default)]
pub struct PenTool {
    anchors: Vec<Anchor>,
    /// True while the button is down on the last anchor, dragging its handles out.
    dragging: bool,
    /// The press that closes the path is in progress.
    closing: bool,
    hover: Option<Point>,
}

impl PenTool {
    fn near_first(&self, ctx: &ToolContext<'_>, p: Point) -> bool {
        self.anchors.len() >= 2 && self.anchors.first().is_some_and(|a| a.pos.distance(p) <= CLOSE_RADIUS / ctx.zoom.max(1.0e-6))
    }

    fn finish(&mut self, ctx: &ToolContext<'_>, closed: bool) -> ToolOutput {
        let anchors = std::mem::take(&mut self.anchors);
        self.dragging = false;
        self.closing = false;
        if anchors.len() < 2 || !ctx.layer_editable(ctx.layer) {
            return ToolOutput::cursor(Cursor::Pen);
        }
        let path = build_path(&anchors, closed);
        let fill = (closed || anchors.len() >= 3).then(|| ctx.settings.active_fill()).flatten();
        let stroke = ctx.settings.active_stroke();
        if fill.is_none() && stroke.is_none() {
            return ToolOutput::cursor(Cursor::Pen);
        }
        let shape = EdgeShape::from_paths(&[PathInput::new(path, fill, stroke)]);
        if shape.is_empty() {
            return ToolOutput::cursor(Cursor::Pen);
        }
        ToolOutput::with(Cursor::Pen, vec![ToolCommand::DrawShape { layer: ctx.layer, frame: ctx.frame, shape, object_drawing: ctx.settings.object_drawing }])
    }
}

impl Tool for PenTool {
    fn id(&self) -> ToolId {
        ToolId::Pen
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => {
                    if p.click_count >= 2 && !self.anchors.is_empty() {
                        // The first click of the double click already placed this anchor.
                        return self.finish(ctx, false);
                    }
                    if self.near_first(ctx, p.pos) {
                        self.closing = true;
                        return ToolOutput::cursor(Cursor::PenClose);
                    }
                    self.anchors.push(Anchor::corner(p.pos));
                    self.dragging = true;
                    ToolOutput::cursor(Cursor::Pen)
                }
                PointerKind::Move => {
                    if self.dragging
                        && let Some(last) = self.anchors.last_mut()
                        && last.pos.distance(p.pos) > ctx.drag_threshold()
                    {
                        last.handle_out = p.pos;
                        last.handle_in = last.pos + (last.pos - p.pos);
                    }
                    self.hover = Some(p.pos);
                    ToolOutput::cursor(Cursor::Pen)
                }
                PointerKind::Up => {
                    self.dragging = false;
                    if self.closing {
                        return self.finish(ctx, true);
                    }
                    ToolOutput::cursor(Cursor::Pen)
                }
                PointerKind::Hover => {
                    self.hover = Some(p.pos);
                    ToolOutput::cursor(if self.near_first(ctx, p.pos) { Cursor::PenClose } else { Cursor::Pen })
                }
            },
            ToolEvent::Key(k) if k.pressed => match k.key {
                Key::Enter => self.finish(ctx, false),
                Key::Escape => {
                    self.cancel();
                    ToolOutput::cursor(Cursor::Pen)
                }
                Key::Backspace | Key::Delete => {
                    self.anchors.pop();
                    ToolOutput::cursor(Cursor::Pen)
                }
                _ => ToolOutput::cursor(Cursor::Pen),
            },
            ToolEvent::Cancel => {
                self.cancel();
                ToolOutput::cursor(Cursor::Pen)
            }
            _ => ToolOutput::cursor(Cursor::Pen),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        if self.anchors.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut preview = self.anchors.clone();
        if !self.dragging
            && let Some(h) = self.hover
        {
            preview.push(Anchor::corner(h));
        }
        out.push(Overlay::Path { path: build_path(&preview, false), role: Role::Preview, width: None, filled: false });
        let last = self.anchors.len().saturating_sub(1);
        let nodes = self
            .anchors
            .iter()
            .enumerate()
            .map(|(i, a)| Node {
                pos: a.pos,
                control_in: (!a.is_corner() && a.handle_in != a.pos).then_some(a.handle_in),
                control_out: (!a.is_corner() && a.handle_out != a.pos).then_some(a.handle_out),
                selected: i == last,
            })
            .collect();
        out.push(Overlay::Nodes { nodes, closed: false, role: Role::Handle });
        if self.near_first_hover(ctx)
            && let Some(first) = self.anchors.first()
        {
            out.push(Overlay::Marker { pos: first.pos, marker: crate::overlay::Marker::Circle, role: Role::Hover, filled: false });
        }
        out
    }

    fn cancel(&mut self) {
        self.anchors.clear();
        self.dragging = false;
        self.closing = false;
        self.hover = None;
    }

    fn is_busy(&self) -> bool {
        !self.anchors.is_empty()
    }
}

impl PenTool {
    fn near_first_hover(&self, ctx: &ToolContext<'_>) -> bool {
        self.hover.is_some_and(|h| self.near_first(ctx, h))
    }
}
