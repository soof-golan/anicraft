//! The Selection tool: pick, marquee, move, duplicate, bend edges, enter groups and symbols.

use anim_core::{Affine, Point, Rect, Vec2};
use anim_scene::Element;
use anim_shape::{EdgeShape, Segment, TWIPS_PER_PX};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, Modifiers, PointerEvent, PointerKind, ToolEvent};
use crate::hit::{candidates, elements_in_rect, pick};
use crate::overlay::{Cursor, Overlay, Role};
use crate::selection::{ElementRef, Selection};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Arrow-key nudge in stage pixels (shift multiplies it by ten).
const NUDGE: f64 = 1.0;

/// An edge of a merge-mode shape the pointer grabbed to bend.
#[derive(Debug, Clone, PartialEq)]
struct Bend {
    target: ElementRef,
    edge: usize,
    segment: usize,
    t: f64,
    original: EdgeShape,
    /// Maps stage coordinates to the shape's own pixels.
    to_local: Affine,
    /// The shape's matrix, to draw the preview on the stage.
    to_stage: Affine,
}

#[derive(Debug, Clone, Default)]
enum State {
    #[default]
    Idle,
    /// The button is down; nothing has moved far enough yet.
    Pressed {
        start: Point,
        hit: Option<ElementRef>,
        movable: bool,
        narrow_on_click: bool,
        additive: bool,
        bend: Option<Box<Bend>>,
    },
    Moving {
        start: Point,
        current: Point,
        modifiers: Modifiers,
    },
    Marquee {
        start: Point,
        current: Point,
        additive: bool,
    },
    Bending {
        bend: Box<Bend>,
        current: Point,
    },
}

/// Selects and moves things.
#[derive(Debug, Clone, Default)]
pub struct SelectionTool {
    state: State,
    hover: Option<ElementRef>,
}

/// Where a segment of an edge must go so that its point at `t` passes through `target` (twips).
fn bent_segment(segment: Segment, t: f64, target: Point) -> Segment {
    let t = t.clamp(0.08, 0.92);
    let s = 1.0 - t;
    match segment {
        Segment::Line { from, to } | Segment::Quad { from, to, .. } => {
            let w = 2.0 * t * s;
            let ctrl = (target.to_vec2() - from.to_vec2() * (s * s) - to.to_vec2() * (t * t)) / w;
            Segment::Quad { from, ctrl: ctrl.to_point(), to }
        }
        Segment::Cubic { from, ctrl1, ctrl2, to } => {
            let b0 = s * s * s;
            let b1 = 3.0 * s * s * t;
            let b2 = 3.0 * s * t * t;
            let b3 = t * t * t;
            let current = from.to_vec2() * b0 + ctrl1.to_vec2() * b1 + ctrl2.to_vec2() * b2 + to.to_vec2() * b3;
            let d = target.to_vec2() - current;
            let denom = b1.mul_add(b1, b2 * b2);
            Segment::Cubic { from, ctrl1: ctrl1 + d * (b1 / denom), ctrl2: ctrl2 + d * (b2 / denom), to }
        }
    }
}

impl Bend {
    fn shape_with(&self, stage_point: Point) -> EdgeShape {
        let mut shape = self.original.clone();
        let local = self.to_local * stage_point;
        let target = Point::new(local.x * TWIPS_PER_PX, local.y * TWIPS_PER_PX);
        if let Some(edge) = shape.edges.get_mut(self.edge)
            && let Some(seg) = edge.segments.get_mut(self.segment)
        {
            *seg = bent_segment(*seg, self.t, target);
            edge.forget_original();
        }
        shape
    }
}

fn constrain(delta: Vec2, modifiers: Modifiers) -> Vec2 {
    if !modifiers.shift {
        return delta;
    }
    let len = delta.hypot();
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (delta.y.atan2(delta.x) / step).round() * step;
    Vec2::new(len * angle.cos(), len * angle.sin())
}

impl SelectionTool {
    fn bend_candidate(ctx: &ToolContext<'_>, hit: &ElementRef, p: Point) -> Option<Box<Bend>> {
        if ctx.selection.contains(hit) {
            return None;
        }
        let resolved = ctx.resolve(hit)?;
        let Element::Shape(shape) = resolved.element else { return None };
        if shape.is_drawing_object {
            return None;
        }
        let to_stage = resolved.local_to_stage();
        let to_local = to_stage.inverse();
        let local = to_local * p;
        let det = to_local.determinant().abs().sqrt();
        let near = shape.shape.nearest_segment(local, ctx.tolerance() * det)?;
        Some(Box::new(Bend { target: hit.clone(), edge: near.edge, segment: near.segment, t: near.t, original: shape.shape.clone(), to_local, to_stage }))
    }

    fn down(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let hit = pick(ctx, p.pos);
        let mut commands = Vec::new();
        if p.click_count >= 2 {
            return Self::double_click(hit.map(|h| h.element), ctx);
        }
        let shift = p.modifiers.shift;
        if let Some(hit) = hit {
            let element = hit.element;
            let bend = Self::bend_candidate(ctx, &element, p.pos);
            let selected = ctx.selection.contains(&element);
            let (movable, narrow_on_click) = if selected && shift {
                commands.push(ToolCommand::SetSelection { selection: ctx.selection.toggled(&element) });
                (false, false)
            } else if selected {
                (true, ctx.selection.items.len() > 1)
            } else if shift {
                commands.push(ToolCommand::SetSelection { selection: ctx.selection.toggled(&element) });
                (true, false)
            } else {
                commands.push(ToolCommand::SetSelection { selection: Selection { items: vec![element.clone()], scope: ctx.selection.scope.clone() } });
                (true, false)
            };
            self.state = State::Pressed { start: p.pos, hit: Some(element), movable, narrow_on_click, additive: shift, bend };
        } else {
            if !shift && !ctx.selection.is_empty() {
                commands.push(ToolCommand::SetSelection { selection: ctx.selection.with_items(Vec::new()) });
            }
            self.state = State::Pressed { start: p.pos, hit: None, movable: false, narrow_on_click: false, additive: shift, bend: None };
        }
        ToolOutput::with(Cursor::Move, commands)
    }

    fn double_click(hit: Option<ElementRef>, ctx: &ToolContext<'_>) -> ToolOutput {
        let Some(element) = hit else {
            let commands = if ctx.selection.scope.is_some() || !matches!(ctx.timeline, anim_scene::TimelineId::Scene(_)) {
                vec![ToolCommand::ExitScope]
            } else {
                Vec::new()
            };
            return ToolOutput::with(Cursor::Default, commands);
        };
        let Some(resolved) = ctx.resolve(&element) else { return ToolOutput::cursor(Cursor::Default) };
        let commands = match resolved.element {
            Element::Group(_) => vec![ToolCommand::EnterGroup { target: element }],
            Element::SymbolInstance(_) => vec![ToolCommand::EnterSymbol { target: element }],
            Element::Text(_) => vec![
                ToolCommand::SetSelection { selection: Selection { items: vec![element], scope: ctx.selection.scope.clone() } },
                ToolCommand::ActivateTool { tool: ToolId::Text },
            ],
            _ => Vec::new(),
        };
        ToolOutput::with(Cursor::Move, commands)
    }

    fn drag(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let threshold = ctx.drag_threshold();
        let state = std::mem::take(&mut self.state);
        self.state = match state {
            State::Pressed { start, hit, movable, narrow_on_click, additive, bend } => {
                if start.distance(p.pos) < threshold {
                    State::Pressed { start, hit, movable, narrow_on_click, additive, bend }
                } else if let Some(bend) = bend {
                    State::Bending { bend, current: p.pos }
                } else if hit.is_some() && movable {
                    State::Moving { start, current: p.pos, modifiers: p.modifiers }
                } else if hit.is_none() {
                    State::Marquee { start, current: p.pos, additive }
                } else {
                    State::Idle
                }
            }
            State::Moving { start, .. } => State::Moving { start, current: p.pos, modifiers: p.modifiers },
            State::Marquee { start, additive, .. } => State::Marquee { start, current: p.pos, additive },
            State::Bending { bend, .. } => State::Bending { bend, current: p.pos },
            State::Idle => State::Idle,
        };
        let cursor = match self.state {
            State::Marquee { .. } => Cursor::Default,
            State::Bending { .. } => Cursor::Bend,
            _ => Cursor::Move,
        };
        ToolOutput::cursor(cursor)
    }

    fn up(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let state = std::mem::take(&mut self.state);
        let mut commands = Vec::new();
        match state {
            State::Pressed { hit: Some(hit), narrow_on_click: true, .. } => {
                commands.push(ToolCommand::SetSelection { selection: Selection { items: vec![hit], scope: ctx.selection.scope.clone() } });
            }
            State::Moving { start, modifiers, .. } => {
                let delta = constrain(p.pos - start, modifiers);
                if delta.hypot() > 1.0e-9 {
                    if modifiers.alt {
                        commands.push(ToolCommand::DuplicateSelection);
                    }
                    commands.push(ToolCommand::TransformSelection { matrix: Affine::translate(delta) });
                }
            }
            State::Marquee { start, additive, .. } => {
                let rect = Rect::from_points(start, p.pos);
                let mut items = elements_in_rect(ctx, rect);
                if additive {
                    let mut merged = ctx.selection.items.clone();
                    for item in items {
                        if !merged.contains(&item) {
                            merged.push(item);
                        }
                    }
                    items = merged;
                }
                commands.push(ToolCommand::SetSelection { selection: ctx.selection.with_items(items) });
            }
            State::Bending { bend, current: _ } => {
                let shape = bend.shape_with(p.pos);
                if shape != bend.original {
                    commands.push(ToolCommand::ReplaceShape { target: bend.target, shape });
                }
            }
            State::Pressed { .. } | State::Idle => {}
        }
        ToolOutput::with(Cursor::Move, commands)
    }

    fn hover(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let hit = pick(ctx, p.pos);
        self.hover = hit.as_ref().map(|h| h.element.clone());
        let cursor = match &hit {
            None => Cursor::Default,
            Some(h) if Self::bend_candidate(ctx, &h.element, p.pos).is_some() => Cursor::Bend,
            Some(_) => Cursor::Move,
        };
        ToolOutput::cursor(cursor)
    }

    fn key(&mut self, key: Key, modifiers: Modifiers, ctx: &ToolContext<'_>) -> ToolOutput {
        let nudge = if modifiers.shift { NUDGE * 10.0 } else { NUDGE };
        let mut commands = Vec::new();
        match key {
            Key::Escape => {
                if self.is_busy() {
                    self.state = State::Idle;
                } else if ctx.selection.scope.is_some() {
                    commands.push(ToolCommand::ExitScope);
                } else if !ctx.selection.is_empty() {
                    commands.push(ToolCommand::SetSelection { selection: Selection::empty() });
                }
            }
            Key::Delete | Key::Backspace if !ctx.selection.is_empty() => commands.push(ToolCommand::DeleteSelection),
            Key::Left | Key::Right | Key::Up | Key::Down if !ctx.selection.is_empty() => {
                let delta = match key {
                    Key::Left => Vec2::new(-nudge, 0.0),
                    Key::Right => Vec2::new(nudge, 0.0),
                    Key::Up => Vec2::new(0.0, -nudge),
                    _ => Vec2::new(0.0, nudge),
                };
                commands.push(ToolCommand::TransformSelection { matrix: Affine::translate(delta) });
            }
            Key::Char('a') if modifiers.ctrl => {
                let items = candidates(ctx).into_iter().map(|c| c.element_ref).collect();
                commands.push(ToolCommand::SetSelection { selection: ctx.selection.with_items(items) });
            }
            Key::Char('d') if modifiers.ctrl && !ctx.selection.is_empty() => commands.push(ToolCommand::DuplicateSelection),
            _ => {}
        }
        ToolOutput::with(Cursor::Default, commands)
    }
}

impl Tool for SelectionTool {
    fn id(&self) -> ToolId {
        ToolId::Selection
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => self.down(p, ctx),
                PointerKind::Move => self.drag(p, ctx),
                PointerKind::Up => self.up(p, ctx),
                PointerKind::Hover => self.hover(p, ctx),
            },
            ToolEvent::Key(k) if k.pressed => self.key(k.key, k.modifiers, ctx),
            ToolEvent::Cancel => {
                self.cancel();
                ToolOutput::none()
            }
            _ => ToolOutput::none(),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let mut out = Vec::new();
        let ghost = match &self.state {
            State::Moving { start, current, modifiers } => Some(constrain(*current - *start, *modifiers)),
            _ => None,
        };
        for item in &ctx.selection.items {
            if let Some(bounds) = ctx.stage_bounds(item) {
                out.push(Overlay::Rect { rect: bounds, role: Role::Selection });
                if let Some(d) = ghost {
                    out.push(Overlay::Rect { rect: bounds + d, role: Role::Preview });
                }
            }
        }
        if let Some(hover) = &self.hover
            && !ctx.selection.contains(hover)
            && let Some(bounds) = ctx.stage_bounds(hover)
        {
            out.push(Overlay::Rect { rect: bounds, role: Role::Hover });
        }
        match &self.state {
            State::Marquee { start, current, .. } => out.push(Overlay::Marquee { rect: Rect::from_points(*start, *current) }),
            State::Bending { bend, current } => {
                let shape = bend.shape_with(*current);
                let assembled = shape.assemble();
                for run in &assembled.strokes {
                    out.push(Overlay::Path { path: bend.to_stage * run.path.clone(), role: Role::Preview, width: None, filled: false });
                }
            }
            _ => {}
        }
        out
    }

    fn cancel(&mut self) {
        self.state = State::Idle;
    }

    fn is_busy(&self) -> bool {
        !matches!(self.state, State::Idle | State::Pressed { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bent_line_passes_through_the_target() {
        let line = Segment::Line { from: Point::new(0.0, 0.0), to: Point::new(200.0, 0.0) };
        let bent = bent_segment(line, 0.5, Point::new(100.0, 40.0));
        let Segment::Quad { ctrl, .. } = bent else { panic!("quad expected") };
        // A quad's midpoint is (from + 2 ctrl + to) / 4.
        let mid = Point::new((0.0 + 2.0 * ctrl.x + 200.0) / 4.0, (0.0 + 2.0 * ctrl.y + 0.0) / 4.0);
        assert!((mid.x - 100.0).abs() < 1.0e-9 && (mid.y - 40.0).abs() < 1.0e-9);
    }

    #[test]
    fn bent_cubic_keeps_its_ends() {
        let c = Segment::Cubic { from: Point::new(0.0, 0.0), ctrl1: Point::new(0.0, 50.0), ctrl2: Point::new(100.0, 50.0), to: Point::new(100.0, 0.0) };
        let bent = bent_segment(c, 0.5, Point::new(50.0, 80.0));
        assert_eq!(bent.start(), c.start());
        assert_eq!(bent.end(), c.end());
        let Segment::Cubic { ctrl1, ctrl2, .. } = bent else { panic!("cubic expected") };
        let mid = Point::new(0.125 * 0.0 + 0.375 * ctrl1.x + 0.375 * ctrl2.x + 0.125 * 100.0, 0.375 * ctrl1.y + 0.375 * ctrl2.y);
        assert!((mid.x - 50.0).abs() < 1.0e-9 && (mid.y - 80.0).abs() < 1.0e-9);
    }
}
