//! The geometry of the Align panel: align, distribute, match size and space.
//!
//! Every function takes the stage bounds of the selected objects and returns one stage-space [`Affine`] per object
//! (the identity when it stays where it is), in the order of the input. [`crate::mcp_layout`] applies them.

use anim_core::{Affine, Rect, Vec2, num};
use serde::{Deserialize, Serialize};

/// Which edge or centre line objects line up on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    Left,
    HorizontalCenter,
    Right,
    Top,
    VerticalCenter,
    Bottom,
}

impl Edge {
    /// 0 for the edges that line objects up in x, 1 in y.
    #[must_use]
    pub const fn axis(self) -> Axis {
        match self {
            Self::Left | Self::HorizontalCenter | Self::Right => Axis::Horizontal,
            Self::Top | Self::VerticalCenter | Self::Bottom => Axis::Vertical,
        }
    }

    /// The coordinate of this edge of `r`.
    #[must_use]
    pub const fn of(self, r: Rect) -> f64 {
        match self {
            Self::Left => r.x0,
            Self::HorizontalCenter => f64::midpoint(r.x0, r.x1),
            Self::Right => r.x1,
            Self::Top => r.y0,
            Self::VerticalCenter => f64::midpoint(r.y0, r.y1),
            Self::Bottom => r.y1,
        }
    }
}

/// The direction objects are spread along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    const fn span(self, r: Rect) -> (f64, f64) {
        match self {
            Self::Horizontal => (r.x0, r.x1),
            Self::Vertical => (r.y0, r.y1),
        }
    }

    fn shift(self, by: f64) -> Affine {
        if by.abs() < 1.0e-12 {
            return Affine::IDENTITY;
        }
        match self {
            Self::Horizontal => Affine::translate(Vec2::new(by, 0.0)),
            Self::Vertical => Affine::translate(Vec2::new(0.0, by)),
        }
    }
}

/// The union of the rectangles.
#[must_use]
pub fn union(items: &[Rect]) -> Option<Rect> {
    items.iter().copied().reduce(|a, b| a.union(b))
}

/// Lines every object up with `edge` of `reference` (the stage, or the bounds of the whole selection).
#[must_use]
pub fn align(items: &[Rect], edge: Edge, reference: Rect) -> Vec<Affine> {
    let target = edge.of(reference);
    items.iter().map(|r| edge.axis().shift(target - edge.of(*r))).collect()
}

/// Spreads the objects so that the `edge` coordinates are evenly spaced.
///
/// With a `reference`, the first and the last object go flush against its two ends; otherwise they stay where they
/// are. Fewer than three objects (without a reference) cannot be distributed.
#[must_use]
pub fn distribute(items: &[Rect], edge: Edge, reference: Option<Rect>) -> Vec<Affine> {
    let axis = edge.axis();
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|a, b| {
        let key = |i: &usize| items.get(*i).map_or(0.0, |r| edge.of(*r));
        key(a).total_cmp(&key(b))
    });
    let mut out = vec![Affine::IDENTITY; items.len()];
    let (Some(&first), Some(&last)) = (order.first(), order.last()) else { return out };
    let (Some(&r_first), Some(&r_last)) = (items.get(first), items.get(last)) else { return out };
    if order.len() < 2 || (reference.is_none() && order.len() < 3) {
        return out;
    }
    let (start, end) = reference.map_or_else(
        || (edge.of(r_first), edge.of(r_last)),
        |stage| {
            let (lo, hi) = axis.span(stage);
            let (first_lo, _) = axis.span(r_first);
            let (_, last_hi) = axis.span(r_last);
            (lo + (edge.of(r_first) - first_lo), hi - (last_hi - edge.of(r_last)))
        },
    );
    let step = (end - start) / num::usize_to_f64(order.len().saturating_sub(1));
    for (n, i) in order.iter().enumerate() {
        let Some(r) = items.get(*i) else { continue };
        let want = num::usize_to_f64(n).mul_add(step, start);
        if let Some(slot) = out.get_mut(*i) {
            *slot = axis.shift(want - edge.of(*r));
        }
    }
    out
}

/// Makes the gaps between neighbouring objects equal along `axis`.
///
/// With a `gap` the objects follow each other at that distance from the first; without one the first and the last
/// stay (or, with `reference`, go flush with its ends) and the gaps are evened out.
#[must_use]
pub fn space(items: &[Rect], axis: Axis, gap: Option<f64>, reference: Option<Rect>) -> Vec<Affine> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|a, b| {
        let key = |i: &usize| items.get(*i).map_or(0.0, |r| axis.span(*r).0);
        key(a).total_cmp(&key(b))
    });
    let mut out = vec![Affine::IDENTITY; items.len()];
    let Some(&first) = order.first() else { return out };
    let Some(&last) = order.last() else { return out };
    if order.len() < 2 {
        return out;
    }
    let (Some(r_first), Some(r_last)) = (items.get(first), items.get(last)) else { return out };
    let sizes: f64 = items.iter().map(|r| axis.span(*r).1 - axis.span(*r).0).sum();
    let (start, end) = match (gap, reference) {
        (None, Some(stage)) => axis.span(stage),
        _ => (axis.span(*r_first).0, axis.span(*r_last).1),
    };
    let between = num::usize_to_f64(order.len().saturating_sub(1));
    let gap = gap.unwrap_or_else(|| (end - start - sizes) / between);
    let mut cursor = start;
    for i in &order {
        let Some(r) = items.get(*i) else { continue };
        let (lo, hi) = axis.span(*r);
        if let Some(slot) = out.get_mut(*i) {
            *slot = axis.shift(cursor - lo);
        }
        cursor += hi - lo + gap;
    }
    out
}

/// Scales every object (about the top left of its own bounds) to the width and/or height of the largest, or of
/// `reference` when given.
#[must_use]
pub fn match_size(items: &[Rect], width: bool, height: bool, reference: Option<Rect>) -> Vec<Affine> {
    let (target_w, target_h) = reference.map_or_else(
        || (items.iter().map(Rect::width).fold(0.0, f64::max), items.iter().map(Rect::height).fold(0.0, f64::max)),
        |r| (r.width(), r.height()),
    );
    items
        .iter()
        .map(|r| {
            let sx: f64 = if width && r.width() > 1.0e-9 { target_w / r.width() } else { 1.0 };
            let sy: f64 = if height && r.height() > 1.0e-9 { target_h / r.height() } else { 1.0 };
            if (sx - 1.0).abs() < 1.0e-12 && (sy - 1.0).abs() < 1.0e-12 {
                return Affine::IDENTITY;
            }
            Affine::translate(Vec2::new(r.x0, r.y0)) * Affine::scale_non_uniform(sx, sy) * Affine::translate(Vec2::new(-r.x0, -r.y0))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moved(r: Rect, m: Affine) -> Rect {
        m.transform_rect_bbox(r)
    }

    fn boxes() -> Vec<Rect> {
        vec![Rect::new(10.0, 10.0, 30.0, 30.0), Rect::new(50.0, 20.0, 100.0, 60.0), Rect::new(200.0, 0.0, 210.0, 10.0)]
    }

    #[test]
    fn align_moves_every_object_to_the_edge() {
        let b = boxes();
        let to = union(&b).unwrap();
        for (r, m) in b.iter().zip(align(&b, Edge::Right, to)) {
            assert!((moved(*r, m).x1 - 210.0).abs() < 1.0e-9);
        }
        for (r, m) in b.iter().zip(align(&b, Edge::VerticalCenter, Rect::new(0.0, 0.0, 550.0, 400.0))) {
            let n = moved(*r, m);
            assert!((f64::midpoint(n.y0, n.y1) - 200.0).abs() < 1.0e-9);
        }
    }

    #[test]
    fn distribute_evens_out_the_left_edges() {
        let b = boxes();
        let m = distribute(&b, Edge::Left, None);
        let lefts: Vec<f64> = b.iter().zip(&m).map(|(r, m)| moved(*r, *m).x0).collect();
        assert!((lefts[0] - 10.0).abs() < 1.0e-9 && (lefts[2] - 200.0).abs() < 1.0e-9);
        assert!((lefts[1] - 105.0).abs() < 1.0e-9, "{lefts:?}");
        assert!(distribute(&b[..2], Edge::Left, None).iter().all(|m| *m == Affine::IDENTITY), "two objects cannot be distributed");
    }

    #[test]
    fn distribute_to_the_stage_goes_flush_with_both_ends() {
        let b = boxes();
        let m = distribute(&b, Edge::Left, Some(Rect::new(0.0, 0.0, 550.0, 400.0)));
        let moved_boxes: Vec<Rect> = b.iter().zip(&m).map(|(r, m)| moved(*r, *m)).collect();
        assert!(moved_boxes[0].x0.abs() < 1.0e-9);
        assert!((moved_boxes[2].x1 - 550.0).abs() < 1.0e-9);
    }

    #[test]
    fn space_makes_the_gaps_equal() {
        let b = boxes();
        let m = space(&b, Axis::Horizontal, None, None);
        let n: Vec<Rect> = b.iter().zip(&m).map(|(r, m)| moved(*r, *m)).collect();
        let g1 = n[1].x0 - n[0].x1;
        let g2 = n[2].x0 - n[1].x1;
        assert!((g1 - g2).abs() < 1.0e-9, "{g1} {g2}");
        assert!((n[0].x0 - 10.0).abs() < 1.0e-9 && (n[2].x1 - 210.0).abs() < 1.0e-9);
        let fixed = space(&b, Axis::Horizontal, Some(5.0), None);
        let f: Vec<Rect> = b.iter().zip(&fixed).map(|(r, m)| moved(*r, *m)).collect();
        assert!((f[1].x0 - f[0].x1 - 5.0).abs() < 1.0e-9 && (f[2].x0 - f[1].x1 - 5.0).abs() < 1.0e-9);
    }

    #[test]
    fn match_size_scales_to_the_largest() {
        let b = boxes();
        let m = match_size(&b, true, true, None);
        for (r, m) in b.iter().zip(&m) {
            let n = moved(*r, *m);
            assert!((n.width() - 50.0).abs() < 1.0e-9 && (n.height() - 40.0).abs() < 1.0e-9, "{n:?}");
            assert!((n.x0 - r.x0).abs() < 1.0e-9 && (n.y0 - r.y0).abs() < 1.0e-9, "anchored at the top left");
        }
        let only_w = match_size(&b, true, false, Some(Rect::new(0.0, 0.0, 20.0, 20.0)));
        assert!((moved(b[1], only_w[1]).width() - 20.0).abs() < 1.0e-9);
        assert!((moved(b[1], only_w[1]).height() - 40.0).abs() < 1.0e-9);
    }
}
