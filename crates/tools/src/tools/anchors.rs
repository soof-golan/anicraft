//! The Add, Delete and Convert Anchor Point tools, and the same edits as pure functions on shapes.
//!
//! Add splits the segment under the pointer in two without changing its curve. Delete joins the neighbours of an
//! anchor with a straight line. Convert turns a corner (no handles) into a smooth point (collinear handles) and a
//! smooth point back into a corner; dragging from an anchor pulls out the handles along the drag.

use anim_core::kurbo::{ParamCurve, ParamCurveNearest, PathSeg};
use anim_core::{Affine, Point, Vec2};
use anim_scene::Element;
use anim_shape::{EdgeShape, Segment, TWIPS_PER_PX};

use super::subselection::{PointKey, delete_anchors};
use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::event::{PointerEvent, PointerKind, ToolEvent};
use crate::overlay::{Cursor, Marker, Overlay, Role};
use crate::selection::ElementRef;
use crate::tool::{Tool, ToolId, ToolOutput};

/// Anchors closer than this many twips are the same anchor.
const SAME_TWIPS: f64 = 0.5;
/// Anchor pick radius in screen points.
const ANCHOR_RADIUS: f64 = 7.0;
/// A handle shorter than this many twips counts as no handle.
const NO_HANDLE: f64 = 0.5;

fn px_to_twips(p: Point) -> Point {
    Point::new(p.x * TWIPS_PER_PX, p.y * TWIPS_PER_PX)
}

/// The segment as a cubic: its four points.
fn cubic_points(segment: Segment) -> [Point; 4] {
    match segment {
        Segment::Line { from, to } => [from, from.lerp(to, 1.0 / 3.0), from.lerp(to, 2.0 / 3.0), to],
        Segment::Quad { from, ctrl, to } => [from, from.lerp(ctrl, 2.0 / 3.0), to.lerp(ctrl, 2.0 / 3.0), to],
        Segment::Cubic { from, ctrl1, ctrl2, to } => [from, ctrl1, ctrl2, to],
    }
}

const fn segment_of(seg: PathSeg) -> Segment {
    match seg {
        PathSeg::Line(l) => Segment::Line { from: l.p0, to: l.p1 },
        PathSeg::Quad(q) => Segment::Quad { from: q.p0, ctrl: q.p1, to: q.p2 },
        PathSeg::Cubic(c) => Segment::Cubic { from: c.p0, ctrl1: c.p1, ctrl2: c.p2, to: c.p3 },
    }
}

/// Every anchor of the shape (twips), once.
fn anchors(shape: &EdgeShape) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::new();
    for segment in shape.edges.iter().flat_map(|e| e.segments.iter()) {
        for p in [segment.start(), segment.end()] {
            if !out.iter().any(|a| a.distance(p) < SAME_TWIPS) {
                out.push(p);
            }
        }
    }
    out
}

/// The anchor of `shape` nearest to `p` within `radius`, both in the shape's pixels. The anchor is in twips.
#[must_use]
pub fn anchor_near(shape: &EdgeShape, p: Point, radius: f64) -> Option<Point> {
    let q = px_to_twips(p);
    let limit = radius * TWIPS_PER_PX;
    anchors(shape).into_iter().map(|a| (a.distance(q), a)).filter(|(d, _)| *d <= limit).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, a)| a)
}

/// Adds an anchor on the segment of `shape` nearest to `p` (shape pixels) within `tolerance` pixels.
///
/// The curve does not change. `None` when nothing is that close or the nearest point already is an anchor.
#[must_use]
pub fn add_anchor(shape: &EdgeShape, p: Point, tolerance: f64) -> Option<EdgeShape> {
    let q = px_to_twips(p);
    let limit = tolerance * TWIPS_PER_PX;
    let mut best: Option<(f64, usize, usize, f64)> = None;
    for (ei, edge) in shape.edges.iter().enumerate() {
        for (si, segment) in edge.segments.iter().enumerate() {
            let near = segment.to_path_seg(1.0).nearest(q, 1.0e-3);
            let d = near.distance_sq.sqrt();
            if d <= limit && best.is_none_or(|(bd, ..)| d < bd) {
                best = Some((d, ei, si, near.t));
            }
        }
    }
    let (_, ei, si, t) = best?;
    let mut out = shape.clone();
    let edge = out.edges.get_mut(ei)?;
    let segment = *edge.segments.get(si)?;
    let seg = segment.to_path_seg(1.0);
    let split_at = seg.eval(t);
    if split_at.distance(segment.start()) < SAME_TWIPS || split_at.distance(segment.end()) < SAME_TWIPS {
        return None;
    }
    let first = segment_of(seg.subsegment(0.0..t));
    let second = segment_of(seg.subsegment(t..1.0));
    edge.segments.splice(si..=si, [first, second]);
    edge.forget_original();
    Some(out)
}

/// True when a curve leaving or entering `anchor` has a handle there.
fn has_handle(segment: Segment, anchor: Point) -> bool {
    match segment {
        Segment::Line { .. } => false,
        Segment::Quad { from, ctrl, to } => (from.distance(anchor) < SAME_TWIPS || to.distance(anchor) < SAME_TWIPS) && ctrl.distance(anchor) > NO_HANDLE,
        Segment::Cubic { from, ctrl1, ctrl2, to } => {
            (from.distance(anchor) < SAME_TWIPS && ctrl1.distance(anchor) > NO_HANDLE)
                || (to.distance(anchor) < SAME_TWIPS && ctrl2.distance(anchor) > NO_HANDLE)
        }
    }
}

/// Where a segment touches the anchor.
#[derive(Debug, Clone, Copy)]
struct Touch {
    edge: usize,
    segment: usize,
}

fn touching(shape: &EdgeShape, anchor: Point) -> (Option<Touch>, Option<Touch>) {
    let mut incoming = None;
    let mut outgoing = None;
    for (edge, e) in shape.edges.iter().enumerate() {
        for (segment, s) in e.segments.iter().enumerate() {
            if s.end().distance(anchor) < SAME_TWIPS && incoming.is_none() {
                incoming = Some(Touch { edge, segment });
            }
            if s.start().distance(anchor) < SAME_TWIPS && outgoing.is_none() {
                outgoing = Some(Touch { edge, segment });
            }
        }
    }
    (incoming, outgoing)
}

fn segment_at(shape: &EdgeShape, t: Touch) -> Option<Segment> {
    shape.edges.get(t.edge)?.segments.get(t.segment).copied()
}

fn put(shape: &mut EdgeShape, t: Touch, segment: Segment) {
    if let Some(edge) = shape.edges.get_mut(t.edge) {
        if let Some(slot) = edge.segments.get_mut(t.segment) {
            *slot = segment;
        }
        edge.forget_original();
    }
}

/// Whether the anchor currently has any handle on a neighbouring curve.
#[must_use]
pub fn is_smooth(shape: &EdgeShape, anchor: Point) -> bool {
    let (incoming, outgoing) = touching(shape, anchor);
    incoming.into_iter().chain(outgoing).filter_map(|t| segment_at(shape, t)).any(|s| has_handle(s, anchor))
}

/// Converts the anchor (twips) of `shape`.
///
/// With `pull` (twips, the drag from the anchor) it becomes smooth with handles along the drag; without, a smooth
/// point becomes a corner and a corner becomes smooth with the tangent between its neighbours. `None` when nothing
/// changes.
#[must_use]
pub fn convert_anchor(shape: &EdgeShape, anchor: Point, pull: Option<Vec2>) -> Option<EdgeShape> {
    let (incoming, outgoing) = touching(shape, anchor);
    let inc = incoming.and_then(|t| segment_at(shape, t));
    let out = outgoing.and_then(|t| segment_at(shape, t));
    if inc.is_none() && out.is_none() {
        return None;
    }
    let to_corner = pull.is_none() && is_smooth(shape, anchor);
    let mut result = shape.clone();
    if to_corner {
        if let (Some(t), Some(s)) = (incoming, inc)
            && has_handle(s, anchor)
        {
            let [a, b, _, d] = cubic_points(s);
            put(&mut result, t, Segment::Cubic { from: a, ctrl1: b, ctrl2: anchor, to: d });
        }
        if let (Some(t), Some(s)) = (outgoing, out)
            && has_handle(s, anchor)
        {
            let [a, _, c, d] = cubic_points(s);
            put(&mut result, t, Segment::Cubic { from: a, ctrl1: anchor, ctrl2: c, to: d });
        }
        return (result != *shape).then_some(result);
    }
    // Smooth: handles along `pull`, or along the line between the neighbours' far points.
    let far_back = inc.map(cubic_points).map(|[a, b, ..]| if b.distance(a) > NO_HANDLE { b } else { a });
    let far_forward = out.map(cubic_points).map(|[.., c, d]| if c.distance(d) > NO_HANDLE { c } else { d });
    let (dir, len_in, len_out) = if let Some(v) = pull {
        let len = v.hypot();
        if len < NO_HANDLE {
            return None;
        }
        (v / len, len, len)
    } else {
        let toward = match (far_back, far_forward) {
            (Some(b), Some(f)) => f - b,
            (Some(b), None) => anchor - b,
            (None, Some(f)) => f - anchor,
            (None, None) => return None,
        };
        let len = toward.hypot();
        if len < 1.0e-9 {
            return None;
        }
        let l_in = inc.map_or(0.0, |s| s.start().distance(anchor) / 3.0);
        let l_out = out.map_or(0.0, |s| s.end().distance(anchor) / 3.0);
        (toward / len, l_in, l_out)
    };
    if let (Some(t), Some(s)) = (incoming, inc) {
        let [a, b, _, d] = cubic_points(s);
        put(&mut result, t, Segment::Cubic { from: a, ctrl1: b, ctrl2: anchor - dir * len_in, to: d });
    }
    if let (Some(t), Some(s)) = (outgoing, out) {
        let [a, _, c, d] = cubic_points(s);
        put(&mut result, t, Segment::Cubic { from: a, ctrl1: anchor + dir * len_out, ctrl2: c, to: d });
    }
    (result != *shape).then_some(result)
}

/// What a click does to the anchors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorMode {
    Add,
    Delete,
    Convert,
}

/// The shape under `pos` (a selected shape first) with its stage-to-local matrix.
fn target(ctx: &ToolContext<'_>, pos: Point, selected_only: bool) -> Option<(ElementRef, EdgeShape, Affine)> {
    let tol = ctx.tolerance();
    let mut candidates: Vec<ElementRef> = ctx.selection.items.clone();
    if !selected_only {
        candidates.extend(crate::hit::candidates(ctx).into_iter().map(|c| c.element_ref));
    }
    for r in candidates {
        let Some(resolved) = ctx.resolve(&r) else { continue };
        let Element::Shape(shape) = resolved.element else { continue };
        let to_stage = resolved.local_to_stage();
        let det = to_stage.determinant();
        if !det.is_finite() || det.abs() < 1.0e-12 {
            continue;
        }
        let to_local = to_stage.inverse();
        let local = to_local * pos;
        let scale = det.abs().sqrt().max(1.0e-9);
        let near_anchor = anchor_near(&shape.shape, local, ANCHOR_RADIUS / ctx.zoom.max(1.0e-6) / scale).is_some();
        let near_edge = shape.shape.edges.iter().flat_map(|e| e.segments.iter()).any(|s| {
            let d = s.to_path_seg(1.0).nearest(px_to_twips(local), 1.0e-3).distance_sq.sqrt();
            d <= tol / scale * TWIPS_PER_PX
        });
        if near_anchor || near_edge {
            return Some((r, shape.shape.clone(), to_local));
        }
    }
    None
}

/// The command that converts `anchor` (twips, in the shape's own space) of the shape element `r`; `pull_stage` is the
/// drag from the anchor in stage pixels.
#[must_use]
pub fn convert_element_anchor(ctx: &ToolContext<'_>, r: &ElementRef, anchor: Point, pull_stage: Option<Vec2>) -> Option<ToolCommand> {
    let resolved = ctx.resolve(r)?;
    let Element::Shape(shape) = resolved.element else { return None };
    let to_stage = resolved.local_to_stage();
    if !to_stage.determinant().is_finite() || to_stage.determinant().abs() < 1.0e-12 {
        return None;
    }
    let to_local = to_stage.inverse();
    let linear = Affine::new([to_local.as_coeffs()[0], to_local.as_coeffs()[1], to_local.as_coeffs()[2], to_local.as_coeffs()[3], 0.0, 0.0]);
    let pull = pull_stage.map(|v| {
        let local = linear * v.to_point();
        Vec2::new(local.x * TWIPS_PER_PX, local.y * TWIPS_PER_PX)
    });
    convert_anchor(&shape.shape, anchor, pull).map(|new| ToolCommand::ReplaceShape { target: r.clone(), shape: new })
}

/// The command that performs the click `mode` at `pos`, if there is something to do.
#[must_use]
pub fn anchor_click(ctx: &ToolContext<'_>, pos: Point, mode: AnchorMode, pull_stage: Option<Vec2>, selected_only: bool) -> Option<ToolCommand> {
    let (r, shape, to_local) = target(ctx, pos, selected_only)?;
    let local = to_local * pos;
    let scale = to_local.determinant().abs().sqrt().max(1.0e-9);
    let radius = ANCHOR_RADIUS / ctx.zoom.max(1.0e-6) * scale;
    let anchor = anchor_near(&shape, local, radius);
    let new = match mode {
        AnchorMode::Add => {
            if anchor.is_some() {
                return None;
            }
            add_anchor(&shape, local, ctx.tolerance() * scale)?
        }
        AnchorMode::Delete => {
            let a = anchor?;
            let cut = delete_anchors(&shape, &[PointKey::Anchor(a)]);
            if cut == shape {
                return None;
            }
            cut
        }
        AnchorMode::Convert => {
            let a = anchor?;
            let linear = Affine::new([to_local.as_coeffs()[0], to_local.as_coeffs()[1], to_local.as_coeffs()[2], to_local.as_coeffs()[3], 0.0, 0.0]);
            let pull = pull_stage.map(|v| {
                let local = linear * v.to_point();
                Vec2::new(local.x * TWIPS_PER_PX, local.y * TWIPS_PER_PX)
            });
            convert_anchor(&shape, a, pull)?
        }
    };
    Some(ToolCommand::ReplaceShape { target: r, shape: new })
}

/// Shared implementation of the three tools.
#[derive(Debug, Clone)]
struct AnchorTool {
    mode: AnchorMode,
    press: Option<Point>,
    current: Point,
}

impl AnchorTool {
    const fn new(mode: AnchorMode) -> Self {
        Self { mode, press: None, current: Point::ZERO }
    }

    fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        let ToolEvent::Pointer(p) = event else {
            if matches!(event, ToolEvent::Cancel) {
                self.press = None;
            }
            return ToolOutput::cursor(Cursor::Point);
        };
        self.down_move_up(p, ctx)
    }

    fn down_move_up(&mut self, p: &PointerEvent, ctx: &ToolContext<'_>) -> ToolOutput {
        match p.kind {
            PointerKind::Down => {
                self.press = Some(p.pos);
                self.current = p.pos;
                ToolOutput::cursor(Cursor::Point)
            }
            PointerKind::Move => {
                self.current = p.pos;
                ToolOutput::cursor(Cursor::Point)
            }
            PointerKind::Up => {
                let Some(start) = self.press.take() else { return ToolOutput::cursor(Cursor::Point) };
                let dragged = start.distance(p.pos) >= ctx.drag_threshold();
                let pull = (self.mode == AnchorMode::Convert && dragged).then(|| p.pos - start);
                let commands = anchor_click(ctx, start, self.mode, pull, false).into_iter().collect();
                ToolOutput::with(Cursor::Point, commands)
            }
            PointerKind::Hover => ToolOutput::cursor(Cursor::Point),
        }
    }

    fn overlay(&self) -> Vec<Overlay> {
        match self.press {
            Some(start) if self.mode == AnchorMode::Convert && start.distance(self.current) > 0.5 => vec![
                Overlay::Line { a: start, b: self.current, role: Role::Guide },
                Overlay::Marker { pos: self.current, marker: Marker::Circle, role: Role::Handle, filled: true },
            ],
            _ => Vec::new(),
        }
    }
}

macro_rules! anchor_tool {
    ($name:ident, $mode:expr, $id:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone)]
        pub struct $name(AnchorTool);

        impl Default for $name {
            fn default() -> Self {
                Self(AnchorTool::new($mode))
            }
        }

        impl Tool for $name {
            fn id(&self) -> ToolId {
                $id
            }

            fn handle(&mut self, event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolOutput {
                self.0.handle(event, ctx)
            }

            fn overlay(&self, _ctx: &ToolContext<'_>) -> Vec<Overlay> {
                self.0.overlay()
            }

            fn cancel(&mut self) {
                self.0.press = None;
            }

            fn is_busy(&self) -> bool {
                self.0.press.is_some()
            }
        }
    };
}

anchor_tool!(AddAnchorTool, AnchorMode::Add, ToolId::AddAnchor, "Adds an anchor point on the path under the pointer.");
anchor_tool!(DeleteAnchorTool, AnchorMode::Delete, ToolId::DeleteAnchor, "Deletes the anchor point under the pointer.");
anchor_tool!(ConvertAnchorTool, AnchorMode::Convert, ToolId::ConvertAnchor, "Converts the anchor under the pointer between corner and smooth.");

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

    fn segment_count(s: &EdgeShape) -> usize {
        s.edges.iter().map(|e| e.segments.len()).sum()
    }

    #[test]
    fn adding_an_anchor_splits_a_segment_without_moving_the_outline() {
        let shape = square();
        let cut = add_anchor(&shape, Point::new(50.0, 0.0), 3.0).expect("anchor added");
        assert_eq!(segment_count(&cut), segment_count(&shape) + 1);
        assert_eq!(cut.bounds(), shape.bounds());
        assert!(anchor_near(&cut, Point::new(50.0, 0.0), 0.5).is_some());
        assert!(cut.fill_at(Point::new(50.0, 50.0)).is_some());
    }

    #[test]
    fn adding_on_an_existing_anchor_or_far_away_does_nothing() {
        let shape = square();
        assert!(add_anchor(&shape, Point::new(0.0, 0.0), 3.0).is_none());
        assert!(add_anchor(&shape, Point::new(50.0, 50.0), 3.0).is_none());
    }

    #[test]
    fn a_corner_becomes_smooth_and_back() {
        let shape = square();
        let corner = Point::new(2000.0, 0.0);
        assert!(!is_smooth(&shape, corner));
        let smooth = convert_anchor(&shape, corner, None).expect("smoothed");
        assert!(is_smooth(&smooth, corner));
        let back = convert_anchor(&smooth, corner, None).expect("cornered");
        assert!(!is_smooth(&back, corner));
    }

    #[test]
    fn dragging_pulls_handles_along_the_drag() {
        let shape = square();
        let corner = Point::new(2000.0, 0.0);
        let pulled = convert_anchor(&shape, corner, Some(Vec2::new(0.0, 400.0))).expect("pulled");
        let (incoming, outgoing) = touching(&pulled, corner);
        let Some(Segment::Cubic { ctrl1, .. }) = outgoing.and_then(|t| segment_at(&pulled, t)) else { panic!("outgoing is not a cubic") };
        assert!((ctrl1.x - corner.x).abs() < 1.0e-9 && (ctrl1.y - corner.y - 400.0).abs() < 1.0e-9);
        let Some(Segment::Cubic { ctrl2, .. }) = incoming.and_then(|t| segment_at(&pulled, t)) else { panic!("incoming is not a cubic") };
        assert!((ctrl2.y - corner.y + 400.0).abs() < 1.0e-9);
    }
}
