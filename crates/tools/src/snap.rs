//! Snapping: the View > Snapping switches (objects, guides, grid, pixels) applied to pointer positions.

use anim_core::Point;
use anim_core::kurbo::ParamCurve as _;
use anim_scene::Element;
use anim_shape::TWIPS_PER_PX;

use crate::context::ToolContext;
use crate::event::{PointerEvent, ToolEvent};
use crate::hit::candidates;
use crate::settings::SnapSettings;
use crate::tool::ToolId;

/// Distance in screen points within which objects and guides attract the pointer.
const REACH: f64 = 8.0;
/// Most shapes whose anchors are searched, so a busy stage does not slow every pointer move.
const MAX_SHAPES: usize = 200;

/// True when the tool takes snapped positions: the ones that place geometry.
#[must_use]
pub const fn tool_snaps(tool: ToolId) -> bool {
    matches!(tool, ToolId::Line | ToolId::Rectangle | ToolId::Oval | ToolId::Pen | ToolId::Pencil | ToolId::Subselection | ToolId::Text)
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
pub fn snap_point(snap: &SnapSettings, ctx: &ToolContext<'_>, p: Point) -> Point {
    if !(snap.objects || snap.guides || snap.grid || snap.pixels) {
        return p;
    }
    let reach = REACH / ctx.zoom.max(1.0e-6);
    if snap.objects {
        let best = anchors(ctx, p, reach).into_iter().filter(|q| q.distance(p) <= reach).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)));
        if let Some(q) = best {
            return q;
        }
    }
    let mut out = p;
    let mut x_done = false;
    let mut y_done = false;
    if snap.guides {
        if let Some(x) = nearest_value(&snap.guides_x, p.x, reach) {
            out.x = x;
            x_done = true;
        }
        if let Some(y) = nearest_value(&snap.guides_y, p.y, reach) {
            out.y = y;
            y_done = true;
        }
    }
    if snap.grid {
        if !x_done {
            let x = round_to(p.x, snap.grid_size);
            if (x - p.x).abs() <= reach {
                out.x = x;
                x_done = true;
            }
        }
        if !y_done {
            let y = round_to(p.y, snap.grid_size);
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
        ToolEvent::Pointer(p) => ToolEvent::Pointer(PointerEvent { pos: snap_point(&ctx.settings.snap, ctx, p.pos), ..*p }),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use anim_scene::Document;

    use super::*;
    use crate::context::{LibraryBounds, ToolContext};
    use crate::selection::Selection;
    use crate::settings::ToolSettings;
    use anim_scene::TimelineId;

    fn with_ctx<R>(settings: &ToolSettings, f: impl FnOnce(&ToolContext<'_>) -> R) -> R {
        let doc = Document::default();
        let bounds = LibraryBounds::new(&doc.library);
        let timeline = TimelineId::Scene(0);
        let selection = Selection::empty();
        let ctx = ToolContext { document: &doc, timeline: &timeline, layer: 0, frame: 0, selection: &selection, settings, zoom: 1.0, bounds: &bounds };
        f(&ctx)
    }

    #[test]
    fn grid_pixels_and_guides_attract_by_axis() {
        let mut settings = ToolSettings::default();
        settings.snap.grid = true;
        settings.snap.grid_size = 20.0;
        with_ctx(&settings, |ctx| {
            assert_eq!(snap_point(&settings.snap, ctx, Point::new(41.0, 18.0)), Point::new(40.0, 20.0));
            assert_eq!(snap_point(&settings.snap, ctx, Point::new(50.0, 50.0)), Point::new(50.0, 50.0), "too far from a grid line");
        });
        settings.snap = SnapSettings { guides: true, guides_x: vec![33.0], pixels: true, ..SnapSettings::default() };
        with_ctx(&settings, |ctx| {
            assert_eq!(snap_point(&settings.snap, ctx, Point::new(35.0, 10.4)), Point::new(33.0, 10.0));
        });
    }

    #[test]
    fn switches_off_leave_the_point_alone() {
        let settings = ToolSettings::default();
        with_ctx(&settings, |ctx| assert_eq!(snap_point(&settings.snap, ctx, Point::new(3.3, 4.4)), Point::new(3.3, 4.4)));
    }
}
