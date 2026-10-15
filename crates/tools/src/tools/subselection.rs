//! The Subselection tool: select and move the points and curve handles of shapes.

use anim_core::{Affine, Point, Rect, Vec2};
use anim_scene::Element;
use anim_shape::{EdgeShape, Segment, TWIPS_PER_PX};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{Key, Modifiers, PointerEvent, PointerKind, ToolEvent};
use crate::hit::pick;
use crate::overlay::{Cursor, Marker, Overlay, Role};
use crate::selection::{ElementRef, Selection};
use crate::tool::{Tool, ToolId, ToolOutput};

/// Point pick radius in screen points.
const POINT_RADIUS: f64 = 6.0;
/// Anchors closer than this many twips are the same anchor.
const SAME_TWIPS: f64 = 0.5;

/// A point of a shape. Anchors are identified by position because neighbouring segments and edges share them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointKey {
    /// An anchor at this position (twips).
    Anchor(Point),
    /// A control point: `which` is 1 or 2 for a cubic's handles, 0 for a quadratic's control.
    Control { edge: usize, segment: usize, which: u8 },
}

#[derive(Debug, Clone, Copy)]
struct ShapePoint {
    key: PointKey,
    /// Position in the shape's pixels.
    pos: Point,
    /// For controls: the anchor the handle hangs on, for drawing the handle line.
    anchor: Option<Point>,
}

fn px(p: Point) -> Point {
    Point::new(p.x / TWIPS_PER_PX, p.y / TWIPS_PER_PX)
}

/// Every anchor and control point of a shape.
fn shape_points(shape: &EdgeShape) -> Vec<ShapePoint> {
    let mut out: Vec<ShapePoint> = Vec::new();
    let add_anchor = |out: &mut Vec<ShapePoint>, p: Point| {
        if !out.iter().any(|s| matches!(s.key, PointKey::Anchor(a) if a.distance(p) < SAME_TWIPS)) {
            out.push(ShapePoint { key: PointKey::Anchor(p), pos: px(p), anchor: None });
        }
    };
    for (edge_index, edge) in shape.edges.iter().enumerate() {
        for (segment_index, segment) in edge.segments.iter().enumerate() {
            add_anchor(&mut out, segment.start());
            add_anchor(&mut out, segment.end());
            match *segment {
                Segment::Line { .. } => {}
                Segment::Quad { from, ctrl, .. } => out.push(ShapePoint {
                    key: PointKey::Control { edge: edge_index, segment: segment_index, which: 0 },
                    pos: px(ctrl),
                    anchor: Some(px(from)),
                }),
                Segment::Cubic { from, ctrl1, ctrl2, to } => {
                    out.push(ShapePoint {
                        key: PointKey::Control { edge: edge_index, segment: segment_index, which: 1 },
                        pos: px(ctrl1),
                        anchor: Some(px(from)),
                    });
                    out.push(ShapePoint {
                        key: PointKey::Control { edge: edge_index, segment: segment_index, which: 2 },
                        pos: px(ctrl2),
                        anchor: Some(px(to)),
                    });
                }
            }
        }
    }
    out
}

/// The shape with `selected` points moved by `delta` pixels. Moving an anchor also moves the cubic handles that
/// hang on it, and every segment that shares the anchor follows.
#[must_use]
pub fn move_points(shape: &EdgeShape, selected: &[PointKey], delta: Vec2) -> EdgeShape {
    let d = Vec2::new(delta.x * TWIPS_PER_PX, delta.y * TWIPS_PER_PX);
    let anchor_moves = |p: Point| selected.iter().any(|k| matches!(k, PointKey::Anchor(a) if a.distance(p) < SAME_TWIPS));
    let control_moves = |edge: usize, segment: usize, which: u8| selected.contains(&PointKey::Control { edge, segment, which });
    let mut out = shape.clone();
    for (ei, edge) in out.edges.iter_mut().enumerate() {
        let mut changed = false;
        for (si, segment) in edge.segments.iter_mut().enumerate() {
            let before = *segment;
            *segment = match before {
                Segment::Line { from, to } => {
                    Segment::Line { from: if anchor_moves(from) { from + d } else { from }, to: if anchor_moves(to) { to + d } else { to } }
                }
                Segment::Quad { from, ctrl, to } => {
                    let (mf, mt) = (anchor_moves(from), anchor_moves(to));
                    let mc = control_moves(ei, si, 0) || (mf && mt);
                    Segment::Quad { from: if mf { from + d } else { from }, ctrl: if mc { ctrl + d } else { ctrl }, to: if mt { to + d } else { to } }
                }
                Segment::Cubic { from, ctrl1, ctrl2, to } => {
                    let (mf, mt) = (anchor_moves(from), anchor_moves(to));
                    Segment::Cubic {
                        from: if mf { from + d } else { from },
                        ctrl1: if mf || control_moves(ei, si, 1) { ctrl1 + d } else { ctrl1 },
                        ctrl2: if mt || control_moves(ei, si, 2) { ctrl2 + d } else { ctrl2 },
                        to: if mt { to + d } else { to },
                    }
                }
            };
            changed |= *segment != before;
        }
        if changed {
            edge.forget_original();
        }
    }
    out
}

/// Removes the selected anchors that sit between two continuous segments of one edge, joining their neighbours with
/// a straight line.
#[must_use]
pub fn delete_anchors(shape: &EdgeShape, selected: &[PointKey]) -> EdgeShape {
    let doomed = |p: Point| selected.iter().any(|k| matches!(k, PointKey::Anchor(a) if a.distance(p) < SAME_TWIPS));
    let mut out = shape.clone();
    for edge in &mut out.edges {
        let mut kept: Vec<Segment> = Vec::with_capacity(edge.segments.len());
        let mut changed = false;
        for segment in &edge.segments {
            match kept.last().copied() {
                Some(prev) if prev.end().distance(segment.start()) < SAME_TWIPS && doomed(segment.start()) => {
                    kept.pop();
                    kept.push(Segment::Line { from: prev.start(), to: segment.end() });
                    changed = true;
                }
                _ => kept.push(*segment),
            }
        }
        if changed {
            edge.segments = kept;
            edge.forget_original();
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
enum State {
    #[default]
    Idle,
    Pressed {
        start: Point,
        on_point: bool,
        additive: bool,
    },
    Moving {
        start: Point,
        current: Point,
    },
    Marquee {
        start: Point,
        current: Point,
        additive: bool,
    },
}

/// Edits the points of shapes.
#[derive(Debug, Clone, Default)]
pub struct SubselectionTool {
    state: State,
    /// Selected points with the element they belong to.
    points: Vec<(ElementRef, PointKey)>,
    /// The selection the points belong to; the points are dropped when it changes.
    for_selection: Vec<ElementRef>,
    hover: Option<Point>,
}

impl SubselectionTool {
    fn sync(&mut self, ctx: &ToolContext<'_>) {
        if self.for_selection != ctx.selection.items {
            self.points.clear();
            self.for_selection.clone_from(&ctx.selection.items);
        }
    }

    /// The point under `p` among the selected shapes.
    fn point_at(ctx: &ToolContext<'_>, p: Point) -> Option<(ElementRef, PointKey)> {
        let radius = POINT_RADIUS / ctx.zoom.max(1.0e-6);
        let mut best: Option<(f64, ElementRef, PointKey)> = None;
        for item in &ctx.selection.items {
            let Some(resolved) = ctx.resolve(item) else { continue };
            let Element::Shape(shape) = resolved.element else { continue };
            let to_stage = resolved.local_to_stage();
            for sp in shape_points(&shape.shape) {
                let d = (to_stage * sp.pos).distance(p);
                // Anchors win over controls at equal distance.
                let d = if matches!(sp.key, PointKey::Anchor(_)) { d } else { d + 1.0e-6 };
                if d <= radius && best.as_ref().is_none_or(|(bd, ..)| d < *bd) {
                    best = Some((d, item.clone(), sp.key));
                }
            }
        }
        best.map(|(_, e, k)| (e, k))
    }

    fn down(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        self.sync(ctx);
        let additive = p.modifiers.shift;
        if let Some((element, key)) = Self::point_at(ctx, p.pos) {
            let entry = (element, key);
            if let Some(i) = self.points.iter().position(|e| *e == entry) {
                if additive {
                    self.points.remove(i);
                }
            } else {
                if !additive {
                    self.points.clear();
                }
                self.points.push(entry);
            }
            self.state = State::Pressed { start: p.pos, on_point: true, additive };
            return ToolOutput::cursor(Cursor::Point);
        }
        let mut commands = Vec::new();
        if let Some(hit) = pick(ctx, p.pos)
            && !ctx.selection.contains(&hit.element)
        {
            commands.push(ToolCommand::SetSelection { selection: Selection { items: vec![hit.element], scope: ctx.selection.scope.clone() } });
        }
        if !additive {
            self.points.clear();
        }
        self.state = State::Pressed { start: p.pos, on_point: false, additive };
        ToolOutput::with(Cursor::Default, commands)
    }

    fn up(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let state = std::mem::take(&mut self.state);
        let mut commands = Vec::new();
        match state {
            State::Moving { start, .. } => {
                let delta = p.pos - start;
                commands = self.replacements(ctx, delta, false);
            }
            State::Marquee { start, additive, .. } => {
                let rect = Rect::from_points(start, p.pos);
                if !additive {
                    self.points.clear();
                }
                for item in &ctx.selection.items {
                    let Some(resolved) = ctx.resolve(item) else { continue };
                    let Element::Shape(shape) = resolved.element else { continue };
                    let to_stage = resolved.local_to_stage();
                    for sp in shape_points(&shape.shape) {
                        let q = to_stage * sp.pos;
                        if q.x >= rect.x0 && q.x <= rect.x1 && q.y >= rect.y0 && q.y <= rect.y1 && !self.points.contains(&(item.clone(), sp.key)) {
                            self.points.push((item.clone(), sp.key));
                        }
                    }
                }
            }
            State::Pressed { .. } | State::Idle => {}
        }
        ToolOutput::with(Cursor::Default, commands)
    }

    /// `ReplaceShape` commands that move (or delete) the selected points.
    fn replacements(&self, ctx: &ToolContext<'_>, stage_delta: Vec2, delete: bool) -> Vec<ToolCommand> {
        let mut commands = Vec::new();
        for item in &ctx.selection.items {
            let keys: Vec<PointKey> = self.points.iter().filter(|(e, _)| e == item).map(|(_, k)| *k).collect();
            if keys.is_empty() {
                continue;
            }
            let Some(resolved) = ctx.resolve(item) else { continue };
            let Element::Shape(shape) = resolved.element else { continue };
            let to_stage = resolved.local_to_stage();
            let linear = Affine::new([to_stage.as_coeffs()[0], to_stage.as_coeffs()[1], to_stage.as_coeffs()[2], to_stage.as_coeffs()[3], 0.0, 0.0]);
            if linear.determinant().abs() < 1.0e-12 {
                continue;
            }
            let new = if delete {
                delete_anchors(&shape.shape, &keys)
            } else {
                move_points(&shape.shape, &keys, (linear.inverse() * stage_delta.to_point()).to_vec2())
            };
            if new != shape.shape {
                commands.push(ToolCommand::ReplaceShape { target: item.clone(), shape: new });
            }
        }
        commands
    }

    fn key(&mut self, key: Key, modifiers: Modifiers, ctx: &ToolContext<'_>) -> ToolOutput {
        self.sync(ctx);
        let step = if modifiers.shift { 10.0 } else { 1.0 };
        let commands = match key {
            Key::Left => self.replacements(ctx, Vec2::new(-step, 0.0), false),
            Key::Right => self.replacements(ctx, Vec2::new(step, 0.0), false),
            Key::Up => self.replacements(ctx, Vec2::new(0.0, -step), false),
            Key::Down => self.replacements(ctx, Vec2::new(0.0, step), false),
            Key::Delete | Key::Backspace => {
                let c = self.replacements(ctx, Vec2::ZERO, true);
                if !c.is_empty() {
                    self.points.clear();
                }
                c
            }
            Key::Escape => {
                self.state = State::Idle;
                self.points.clear();
                Vec::new()
            }
            _ => Vec::new(),
        };
        ToolOutput::with(Cursor::Default, commands)
    }
}

impl Tool for SubselectionTool {
    fn id(&self) -> ToolId {
        ToolId::Subselection
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match event {
            ToolEvent::Pointer(p) => match p.kind {
                PointerKind::Down => self.down(p, ctx),
                PointerKind::Move => {
                    let threshold = ctx.drag_threshold();
                    let state = std::mem::take(&mut self.state);
                    self.state = match state {
                        State::Pressed { start, on_point, additive } if start.distance(p.pos) >= threshold => {
                            if on_point && !self.points.is_empty() {
                                State::Moving { start, current: p.pos }
                            } else {
                                State::Marquee { start, current: p.pos, additive }
                            }
                        }
                        State::Moving { start, .. } => State::Moving { start, current: p.pos },
                        State::Marquee { start, additive, .. } => State::Marquee { start, current: p.pos, additive },
                        other => other,
                    };
                    ToolOutput::cursor(Cursor::Point)
                }
                PointerKind::Up => self.up(p, ctx),
                PointerKind::Hover => {
                    self.hover = Some(p.pos);
                    let over_point = Self::point_at(ctx, p.pos).is_some();
                    ToolOutput::cursor(if over_point { Cursor::Point } else { Cursor::Default })
                }
            },
            ToolEvent::Key(k) if k.pressed => self.key(k.key, k.modifiers, ctx),
            ToolEvent::Cancel => {
                self.state = State::Idle;
                ToolOutput::none()
            }
            _ => ToolOutput::none(),
        }
    }

    fn overlay(&self, ctx: &ToolContext<'_>) -> Vec<Overlay> {
        let mut out = Vec::new();
        let delta = match &self.state {
            State::Moving { start, current } => *current - *start,
            _ => Vec2::ZERO,
        };
        for item in &ctx.selection.items {
            let Some(resolved) = ctx.resolve(item) else { continue };
            let Element::Shape(shape) = resolved.element else { continue };
            let to_stage = resolved.local_to_stage();
            let keys: Vec<PointKey> = self.points.iter().filter(|(e, _)| e == item).map(|(_, k)| *k).collect();
            let shown = if delta == Vec2::ZERO || keys.is_empty() {
                shape.shape.clone()
            } else {
                let linear = Affine::new([to_stage.as_coeffs()[0], to_stage.as_coeffs()[1], to_stage.as_coeffs()[2], to_stage.as_coeffs()[3], 0.0, 0.0]);
                if linear.determinant().abs() < 1.0e-12 {
                    shape.shape.clone()
                } else {
                    move_points(&shape.shape, &keys, (linear.inverse() * delta.to_point()).to_vec2())
                }
            };
            if delta != Vec2::ZERO {
                for fill in shown.assemble().fills {
                    out.push(Overlay::Path { path: to_stage * fill.path, role: Role::Preview, width: None, filled: false });
                }
            }
            // Moving points keeps the structure, so the original and moved lists correspond one to one.
            let original = shape_points(&shape.shape);
            for (before, sp) in original.iter().zip(shape_points(&shown)) {
                let selected = keys.iter().any(|k| match (k, &before.key) {
                    (PointKey::Anchor(a), PointKey::Anchor(b)) => a.distance(*b) < SAME_TWIPS,
                    (a, b) => a == b,
                });
                let pos = to_stage * sp.pos;
                if let Some(anchor) = sp.anchor {
                    out.push(Overlay::Line { a: to_stage * anchor, b: pos, role: Role::Guide });
                    out.push(Overlay::Marker { pos, marker: Marker::Circle, role: Role::Handle, filled: selected });
                } else {
                    out.push(Overlay::Marker { pos, marker: Marker::Square, role: Role::Handle, filled: selected });
                }
            }
        }
        if let State::Marquee { start, current, .. } = &self.state {
            out.push(Overlay::Marquee { rect: Rect::from_points(*start, *current) });
        }
        out
    }

    fn cancel(&mut self) {
        self.state = State::Idle;
    }

    fn is_busy(&self) -> bool {
        matches!(self.state, State::Moving { .. } | State::Marquee { .. })
    }
}

#[cfg(test)]
mod tests {
    use anim_core::{BezPath, Rgba8};
    use anim_shape::{FillStyle, PathInput};

    use super::*;

    fn square() -> EdgeShape {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p.line_to((100.0, 100.0));
        p.line_to((0.0, 100.0));
        p.close_path();
        EdgeShape::from_paths(&[PathInput::new(p, Some(FillStyle::Solid(Rgba8::BLACK)), None)])
    }

    #[test]
    fn anchors_are_listed_once() {
        let pts = shape_points(&square());
        assert_eq!(pts.iter().filter(|p| matches!(p.key, PointKey::Anchor(_))).count(), 4);
    }

    #[test]
    fn moving_a_corner_moves_both_adjacent_edges() {
        let shape = square();
        let corner = PointKey::Anchor(Point::new(2000.0, 2000.0));
        let moved = move_points(&shape, &[corner], Vec2::new(10.0, 5.0));
        let b = moved.bounds().expect("bounds");
        assert!((b.x1 - 110.0).abs() < 1.0e-9 && (b.y1 - 105.0).abs() < 1.0e-9);
        assert!((b.x0).abs() < 1.0e-9 && (b.y0).abs() < 1.0e-9);
        assert!(moved.fill_at(Point::new(95.0, 95.0)).is_some());
    }

    #[test]
    fn deleting_an_anchor_joins_its_neighbours() {
        let shape = square();
        let corner = PointKey::Anchor(Point::new(2000.0, 0.0));
        let cut = delete_anchors(&shape, &[corner]);
        let segments: usize = cut.edges.iter().map(|e| e.segments.len()).sum();
        assert_eq!(segments, 3);
        assert!(cut.fill_at(Point::new(10.0, 60.0)).is_some());
        assert!(cut.fill_at(Point::new(90.0, 10.0)).is_none(), "the corner is cut off");
    }
}
