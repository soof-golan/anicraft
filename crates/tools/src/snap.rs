//! Snapping: the View > Snapping switches (objects, guides, grid, pixels, smart guides, snap align) applied to
//! pointer positions and to moved selections.
//!
//! Guides and the grid belong to the document ([`crate::guides`]); the switches belong to the tool settings.

use anim_core::kurbo::ParamCurve as _;
use anim_core::{Point, Rect, Vec2};
use anim_scene::Element;
use anim_shape::TWIPS_PER_PX;

use crate::context::ToolContext;
use crate::event::{PointerEvent, ToolEvent};
use crate::guides;
use crate::hit::candidates;
use crate::overlay::{Overlay, Role};
use crate::selection::ElementRef;
use crate::tool::ToolId;

/// Most shapes whose anchors are searched, so a busy stage does not slow every pointer move.
const MAX_SHAPES: usize = 200;

/// True when the tool takes snapped positions: the ones that place geometry.
#[must_use]
pub const fn tool_snaps(tool: ToolId) -> bool {
    matches!(
        tool,
        ToolId::Line
            | ToolId::Rectangle
            | ToolId::Oval
            | ToolId::RectanglePrimitive
            | ToolId::OvalPrimitive
            | ToolId::PolyStar
            | ToolId::Pen
            | ToolId::Pencil
            | ToolId::Subselection
            | ToolId::Text
    )
}

/// The reach of snapping in stage pixels.
fn reach(ctx: &ToolContext<'_>) -> f64 {
    ctx.settings.snap.accuracy.points() / ctx.zoom.max(1.0e-6)
}

/// The anchor points (segment ends) of the shapes on the stage, in stage coordinates.
fn anchors(ctx: &ToolContext<'_>, near: Point, reach: f64) -> Vec<Point> {
    let mut out = Vec::new();
    for candidate in candidates(ctx).into_iter().take(MAX_SHAPES) {
        let Element::Shape(s) = candidate.element else { continue };
        let to_stage = candidate.parent_to_stage * s.matrix;
        for edge in &s.shape.edges {
            for segment in &edge.segments {
                let seg = segment.to_path_seg(1.0 / TWIPS_PER_PX);
                for p in [seg.start(), seg.end()] {
                    let q = to_stage * p;
                    if (q.x - near.x).abs() <= reach && (q.y - near.y).abs() <= reach {
                        out.push(q);
                    }
                }
            }
        }
    }
    out
}

fn nearest_value(values: &[f64], at: f64, reach: f64) -> Option<f64> {
    values.iter().copied().filter(|v| (v - at).abs() <= reach).min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs()))
}

fn round_to(value: f64, step: f64) -> f64 {
    if step > 0.0 { (value / step).round() * step } else { value }
}

/// The position `p` snaps to under the switches, or `p` itself.
#[must_use]
pub fn snap_point(ctx: &ToolContext<'_>, p: Point) -> Point {
    let snap = &ctx.settings.snap;
    if !(snap.objects || snap.guides || snap.grid || snap.pixels) {
        return p;
    }
    let reach = reach(ctx);
    if snap.objects {
        let best = anchors(ctx, p, reach).into_iter().filter(|q| q.distance(p) <= reach).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)));
        if let Some(q) = best {
            return q;
        }
    }
    let layout = guides::read(ctx.document);
    let mut out = p;
    let mut x_done = false;
    let mut y_done = false;
    if snap.guides {
        if let Some(x) = nearest_value(&layout.guides_x, p.x, reach) {
            out.x = x;
            x_done = true;
        }
        if let Some(y) = nearest_value(&layout.guides_y, p.y, reach) {
            out.y = y;
            y_done = true;
        }
    }
    if snap.grid {
        if !x_done {
            let x = round_to(p.x, layout.grid[0]);
            if (x - p.x).abs() <= reach {
                out.x = x;
                x_done = true;
            }
        }
        if !y_done {
            let y = round_to(p.y, layout.grid[1]);
            if (y - p.y).abs() <= reach {
                out.y = y;
                y_done = true;
            }
        }
    }
    if snap.pixels {
        if !x_done {
            out.x = p.x.round();
        }
        if !y_done {
            out.y = p.y.round();
        }
    }
    out
}

/// The event with its pointer position snapped (other events are returned as they are).
#[must_use]
pub fn snap_event(event: &ToolEvent, ctx: &ToolContext<'_>) -> ToolEvent {
    match event {
        ToolEvent::Pointer(p) => ToolEvent::Pointer(PointerEvent { pos: snap_point(ctx, p.pos), ..*p }),
        other => other.clone(),
    }
}

// ---- moving a selection -------------------------------------------------------------------------------------------

/// A line a moving box can lock on to.
#[derive(Debug, Clone, Copy)]
struct Target {
    value: f64,
    /// Which of the box's min edge (0), centre (1) or max edge (2) the line attracts; any when `None`.
    only: Option<usize>,
    /// Where the line is drawn along the other axis, when it is a smart guide.
    extent: Option<(f64, f64)>,
}

/// The result of snapping a move.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveSnap {
    /// The (possibly adjusted) distance to move by.
    pub delta: Vec2,
    /// Smart guide lines to draw while the move is in progress.
    pub overlay: Vec<Overlay>,
}

const fn span(r: Rect, horizontal: bool) -> (f64, f64) {
    if horizontal { (r.x0, r.x1) } else { (r.y0, r.y1) }
}

fn is_inside_selection(ctx: &ToolContext<'_>, r: &ElementRef) -> bool {
    ctx.selection.items.iter().any(|s| r.is_within(s))
}

/// The best correction along `axis` for the box `moving` (already shifted) and the lines, with the lines that then
/// coincide with it.
fn solve(moving: (f64, f64), targets: &[Target], reach: f64) -> Option<(f64, Vec<Target>)> {
    let mover = [moving.0, f64::midpoint(moving.0, moving.1), moving.1];
    let mut best: Option<f64> = None;
    for t in targets {
        for (i, m) in mover.iter().enumerate() {
            if t.only.is_some_and(|o| o != i) {
                continue;
            }
            let d = t.value - m;
            if d.abs() <= reach && best.is_none_or(|b| d.abs() < b.abs()) {
                best = Some(d);
            }
        }
    }
    let d = best?;
    let hit: Vec<Target> = targets
        .iter()
        .copied()
        .filter(|t| mover.iter().enumerate().any(|(i, m)| t.only.is_none_or(|o| o == i) && (t.value - (m + d)).abs() < 1.0e-6))
        .collect();
    Some((d, hit))
}

/// Snaps the move of a selection whose stage bounds are `bounds` by `delta`.
///
/// The box's edges and centre lock on to guides, grid lines, the edges (and with smart guides the centres) of other
/// objects, the stage edges and centre with snap align, and whole pixels. Smart guides come back as overlay lines.
#[must_use]
pub fn snap_move(ctx: &ToolContext<'_>, bounds: Rect, delta: Vec2) -> MoveSnap {
    let snap = &ctx.settings.snap;
    let mut out = MoveSnap { delta, overlay: Vec::new() };
    if !(snap.objects || snap.guides || snap.grid || snap.pixels || snap.smart_guides || snap.align) {
        return out;
    }
    let reach = reach(ctx);
    let moved = bounds + delta;
    let layout = guides::read(ctx.document);
    let others: Vec<Rect> = if snap.objects || snap.smart_guides || snap.align {
        candidates(ctx).into_iter().take(MAX_SHAPES).filter(|c| !is_inside_selection(ctx, &c.element_ref)).filter_map(|c| ctx.stage_bounds(&c.element_ref)).collect()
    } else {
        Vec::new()
    };
    let stage = [ctx.document.width, ctx.document.height];
    let mut adjust = [0.0, 0.0];
    let mut lines: Vec<(bool, Target)> = Vec::new();
    for (axis, horizontal) in [true, false].into_iter().enumerate() {
        let (lo, hi) = span(moved, horizontal);
        let across = span(moved, !horizontal);
        let mut targets: Vec<Target> = Vec::new();
        if snap.guides {
            let values = if horizontal { &layout.guides_x } else { &layout.guides_y };
            targets.extend(values.iter().map(|v| Target { value: *v, only: None, extent: None }));
        }
        if snap.grid {
            let step = layout.grid.get(axis).copied().unwrap_or(18.0);
            for (i, m) in [lo, f64::midpoint(lo, hi), hi].into_iter().enumerate() {
                targets.push(Target { value: round_to(m, step), only: Some(i), extent: None });
            }
        }
        for other in &others {
            let (olo, ohi) = span(*other, horizontal);
            let oacross = span(*other, !horizontal);
            let extent = snap.smart_guides.then_some((oacross.0.min(across.0), oacross.1.max(across.1)));
            if snap.objects || snap.smart_guides {
                targets.push(Target { value: olo, only: None, extent });
                targets.push(Target { value: ohi, only: None, extent });
            }
            if snap.smart_guides {
                targets.push(Target { value: f64::midpoint(olo, ohi), only: None, extent });
            }
            if snap.align {
                let gap = snap.object_spacing.get(axis).copied().unwrap_or(0.0);
                targets.push(Target { value: ohi + gap, only: Some(0), extent: None });
                targets.push(Target { value: olo - gap, only: Some(2), extent: None });
            }
        }
        if snap.align {
            let margin = snap.edge_margin.get(axis).copied().unwrap_or(0.0);
            let size = stage.get(axis).copied().unwrap_or(0.0);
            targets.push(Target { value: margin, only: Some(0), extent: None });
            targets.push(Target { value: size - margin, only: Some(2), extent: None });
            if snap.center_align {
                targets.push(Target { value: size / 2.0, only: Some(1), extent: None });
            }
        }
        match solve((lo, hi), &targets, reach) {
            Some((d, hit)) => {
                if let Some(a) = adjust.get_mut(axis) {
                    *a = d;
                }
                lines.extend(hit.into_iter().filter(|t| t.extent.is_some()).map(|t| (horizontal, t)));
            }
            None if snap.pixels => {
                if let Some(a) = adjust.get_mut(axis) {
                    *a = lo.round() - lo;
                }
            }
            None => {}
        }
    }
    let [ax, ay] = adjust;
    out.delta = Vec2::new(delta.x + ax, delta.y + ay);
    let moved = bounds + out.delta;
    for (horizontal, t) in lines {
        let Some((from, to)) = t.extent else { continue };
        let (a, b) = if horizontal {
            (Point::new(t.value, from.min(moved.y0)), Point::new(t.value, to.max(moved.y1)))
        } else {
            (Point::new(from.min(moved.x0), t.value), Point::new(to.max(moved.x1), t.value))
        };
        out.overlay.push(Overlay::Line { a, b, role: Role::Guide });
    }
    out
}

#[cfg(test)]
mod tests {
    use anim_scene::Document;

    use super::*;
    use crate::context::{LibraryBounds, ToolContext};
    use crate::selection::Selection;
    use crate::settings::{SnapSettings, ToolSettings};
    use anim_scene::TimelineId;

    fn with_ctx<R>(settings: &ToolSettings, doc: &Document, f: impl FnOnce(&ToolContext<'_>) -> R) -> R {
        let bounds = LibraryBounds::new(&doc.library);
        let timeline = TimelineId::Scene(0);
        let selection = Selection::empty();
        let ctx = ToolContext {
            document: doc,
            timeline: &timeline,
            layer: 0,
            frame: 0,
            selection: &selection,
            settings,
            zoom: 1.0,
            bounds: &bounds,
            edit_range: None,
        };
        f(&ctx)
    }

    #[test]
    fn grid_pixels_and_guides_attract_by_axis() {
        let mut settings = ToolSettings::default();
        let mut doc = Document::default();
        settings.snap.grid = true;
        guides::write(&mut doc, &guides::Layout { grid: [20.0, 20.0], ..guides::Layout::default() });
        with_ctx(&settings, &doc, |ctx| {
            assert_eq!(snap_point(ctx, Point::new(41.0, 18.0)), Point::new(40.0, 20.0));
            assert_eq!(snap_point(ctx, Point::new(50.0, 50.0)), Point::new(50.0, 50.0), "too far from a grid line");
        });
        settings.snap = SnapSettings { guides: true, pixels: true, ..SnapSettings::default() };
        guides::write(&mut doc, &guides::Layout { guides_x: vec![33.0], ..guides::Layout::default() });
        with_ctx(&settings, &doc, |ctx| {
            assert_eq!(snap_point(ctx, Point::new(35.0, 10.4)), Point::new(33.0, 10.0));
        });
    }

    #[test]
    fn switches_off_leave_the_point_alone() {
        let settings = ToolSettings::default();
        with_ctx(&settings, &Document::default(), |ctx| assert_eq!(snap_point(ctx, Point::new(3.3, 4.4)), Point::new(3.3, 4.4)));
    }

    #[test]
    fn a_moved_box_locks_on_to_guides_the_grid_and_the_stage() {
        let mut settings = ToolSettings::default();
        let mut doc = Document::default();
        let bounds = Rect::new(100.0, 100.0, 140.0, 120.0);
        settings.snap.guides = true;
        guides::write(&mut doc, &guides::Layout { guides_x: vec![205.0], ..guides::Layout::default() });
        with_ctx(&settings, &doc, |ctx| {
            // The box moves 62 to the right: its right edge ends at 202, 3 away from the guide.
            let s = snap_move(ctx, bounds, Vec2::new(62.0, 0.0));
            assert!((s.delta.x - 65.0).abs() < 1.0e-9, "{s:?}");
            assert!(s.delta.y.abs() < 1.0e-9);
        });
        settings.snap = SnapSettings { align: true, center_align: true, edge_margin: [10.0, 10.0], ..SnapSettings::default() };
        with_ctx(&settings, &Document::default(), |ctx| {
            // Moving to x = 7 puts the left edge 3 from the 10 px margin.
            let s = snap_move(ctx, bounds, Vec2::new(-93.0, -95.0));
            assert!((s.delta.x + 90.0).abs() < 1.0e-9 && (s.delta.y + 90.0).abs() < 1.0e-9, "{s:?}");
        });
    }

    #[test]
    fn nothing_is_adjusted_beyond_the_reach() {
        let mut settings = ToolSettings::default();
        settings.snap.guides = true;
        let mut doc = Document::default();
        guides::write(&mut doc, &guides::Layout { guides_x: vec![500.0], ..guides::Layout::default() });
        with_ctx(&settings, &doc, |ctx| {
            let s = snap_move(ctx, Rect::new(0.0, 0.0, 10.0, 10.0), Vec2::new(5.0, 5.0));
            assert_eq!(s.delta, Vec2::new(5.0, 5.0));
            assert_eq!(s.overlay.len(), 0);
        });
    }
}
