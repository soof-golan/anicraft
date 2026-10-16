//! The Width tool: drag out from a stroke to make it wider (or narrower) at that place.
//!
//! Each drag adds a width point at the place where it started. The stroke becomes the filled outline of a line
//! whose width follows the points (see [`crate::widthline`]). While the tool stays on the same shape it remembers
//! the original centre line and the points, so further drags refine the same profile instead of starting over.

use anim_core::{Affine, Point};
use anim_scene::Element;
use anim_shape::EdgeShape;

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, PointerEvent, PointerKind, ToolEvent};
use crate::hit::pick;
use crate::overlay::{Cursor, Marker, Overlay, Role};
use crate::selection::ElementRef;
use crate::tool::{Tool, ToolId, ToolOutput};
use crate::widthline::{WidthProfile, bake, base_width, stroke_hit};

/// A stroke being given a profile.
#[derive(Debug, Clone)]
struct Target {
    element: ElementRef,
    /// The shape as it was before any width was applied.
    original: EdgeShape,
    stroke: usize,
    subpath: usize,
    profile: WidthProfile,
    base: f64,
}

#[derive(Debug, Clone)]
struct Press {
    target: Target,
    /// Where along the sub-path the drag began.
    t: f64,
    /// The place on the stroke, in stage pixels.
    anchor: Point,
    /// Local units per stage pixel.
    local_per_stage: f64,
    current: Point,
}

/// Gives strokes a variable width.
#[derive(Debug, Clone, Default)]
pub struct WidthTool {
    press: Option<Press>,
    session: Option<Target>,
}

impl WidthTool {
    fn down(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let Some(hit) = pick(ctx, p.pos) else {
            self.session = None;
            return ToolOutput::cursor(Cursor::Crosshair);
        };
        let Some(resolved) = ctx.resolve(&hit.element) else { return ToolOutput::cursor(Cursor::Crosshair) };
        let Element::Shape(shape) = resolved.element else {
            self.session = None;
            return ToolOutput::cursor(Cursor::Crosshair);
        };
        let to_stage: Affine = resolved.local_to_stage();
        let det = to_stage.determinant();
        if !det.is_finite() || det.abs() < 1.0e-12 {
            return ToolOutput::cursor(Cursor::Crosshair);
        }
        let scale = det.abs().sqrt();
        let local = to_stage.inverse() * p.pos;
        let continuing = self.session.as_ref().filter(|s| s.element == hit.element);
        let (original, tolerance) =
            continuing.map_or_else(|| (shape.shape.clone(), ctx.tolerance() / scale), |s| (s.original.clone(), s.base.mul_add(3.0, ctx.tolerance() / scale)));
        let Some(found) = stroke_hit(&original, local, tolerance) else {
            self.session = None;
            return ToolOutput::cursor(Cursor::Crosshair);
        };
        let base = base_width(&original, found.stroke).unwrap_or(1.0);
        let profile = continuing.filter(|s| s.stroke == found.stroke && s.subpath == found.subpath).map_or(WidthProfile::Uniform, |s| s.profile.clone());
        let target = Target { element: hit.element, original, stroke: found.stroke, subpath: found.subpath, profile, base };
        self.press = Some(Press { target, t: found.t, anchor: to_stage * found.point, local_per_stage: 1.0 / scale, current: p.pos });
        ToolOutput::cursor(Cursor::Crosshair)
    }

    /// The profile and the baked shape a drag to `current` would give.
    fn preview(press: &Press) -> Option<(WidthProfile, EdgeShape)> {
        let width = 2.0 * press.current.distance(press.anchor) * press.local_per_stage;
        let profile = press.target.profile.with_point(press.t, width, press.target.base);
        let baked = bake(&press.target.original, Some(press.target.stroke), Some(press.target.subpath), &profile)?;
        Some((profile, baked))
    }

    fn up(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let Some(mut press) = self.press.take() else { return ToolOutput::cursor(Cursor::Crosshair) };
        press.current = p.pos;
        if press.anchor.distance(p.pos) < ctx.drag_threshold() {
            self.session = Some(press.target);
            return ToolOutput::cursor(Cursor::Crosshair);
        }
        let Some((profile, shape)) = Self::preview(&press) else { return ToolOutput::cursor(Cursor::Crosshair) };
        let target = press.target;
        let command = ToolCommand::ReplaceShape { target: target.element.clone(), shape };
        self.session = Some(Target { profile, ..target });
        ToolOutput::with(Cursor::Crosshair, vec![command])
    }
}

impl Tool for WidthTool {
    fn id(&self) -> ToolId {
        ToolId::Width
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => self.down(p, ctx),
                PointerKind::Move => {
                    if let Some(press) = &mut self.press {
                        press.current = p.pos;
                    }
                    ToolOutput::cursor(Cursor::Crosshair)
                }
                PointerKind::Up => self.up(p, ctx),
                PointerKind::Hover => ToolOutput::cursor(Cursor::Crosshair),
            },
            ToolEvent::Key(k) if k.pressed && k.key == Key::Escape => {
                self.cancel();
                ToolOutput::cursor(Cursor::Crosshair)
            }
            ToolEvent::Cancel => {
                self.press = None;
                ToolOutput::cursor(Cursor::Crosshair)
            }
            _ => ToolOutput::cursor(Cursor::Crosshair),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let Some(press) = &self.press else { return Vec::new() };
        let mut out = vec![
            Overlay::Marker { pos: press.anchor, marker: Marker::Diamond, role: Role::Handle, filled: true },
            Overlay::Line { a: press.anchor, b: press.current, role: Role::Guide },
        ];
        if press.anchor.distance(press.current) >= ctx.drag_threshold()
            && let Some((_, baked)) = Self::preview(press)
            && let Some(resolved) = ctx.resolve(&press.target.element)
        {
            let to_stage = resolved.local_to_stage();
            for fill in baked.assemble().fills {
                out.push(Overlay::Path { path: to_stage * fill.path, role: Role::Preview, width: None, filled: false });
            }
        }
        out
    }

    fn cancel(&mut self) {
        self.press = None;
        self.session = None;
    }

    fn is_busy(&self) -> bool {
        self.press.is_some()
    }
}
