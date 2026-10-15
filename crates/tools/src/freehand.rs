//! Geometry for the freehand tools: curve fitting for the pencil and variable-width outlines for the brush.
//!
//! Curve fitting and boolean union come from the `VectorCraft` path operations; this module only adapts them.

use std::f64::consts::{FRAC_PI_2, TAU};

use anim_core::kurbo::{Circle, Shape as _};
use anim_core::num::round_f64_to_i32;
use anim_core::{BezPath, Point, Vec2};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops::{SimplifyOptions, simplify_with, unite_all};

use crate::settings::PencilMode;

/// A pointer sample of a brush stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushSample {
    pub pos: Point,
    /// Radius in stage pixels.
    pub radius: f64,
}

fn unit(angle: f64) -> Vec2 {
    Vec2::new(angle.cos(), angle.sin())
}

/// Appends the arc of a circle as cubic curves, starting at the current point (which must be the arc's start).
fn arc_to(path: &mut BezPath, center: Point, radius: f64, start: f64, sweep: f64) {
    let pieces = usize::try_from(round_f64_to_i32((sweep.abs() / FRAC_PI_2).ceil()).max(1)).unwrap_or(1);
    let delta = sweep / anim_core::num::usize_to_f64(pieces);
    let k = 4.0 / 3.0 * (delta / 4.0).tan();
    let mut a = start;
    for _ in 0..pieces {
        let b = a + delta;
        let p0 = center + unit(a) * radius;
        let p3 = center + unit(b) * radius;
        let t0 = Vec2::new(-a.sin(), a.cos());
        let t3 = Vec2::new(-b.sin(), b.cos());
        path.curve_to(p0 + t0 * (k * radius), p3 - t3 * (k * radius), p3);
        a = b;
    }
}

/// The convex hull of two circles (a stroke segment with round caps and linearly changing width).
#[must_use]
pub fn capsule(c1: Point, r1: f64, c2: Point, r2: f64) -> BezPath {
    let d = c2 - c1;
    let len = d.hypot();
    let (big_c, big_r) = if r1 >= r2 { (c1, r1) } else { (c2, r2) };
    if len <= (r1 - r2).abs() + 1.0e-9 {
        return Circle::new(big_c, big_r).to_path(0.01);
    }
    let theta = d.y.atan2(d.x);
    let phi = ((r1 - r2) / len).clamp(-1.0, 1.0).acos();
    let mut path = BezPath::new();
    path.move_to(c1 + unit(theta - phi) * r1);
    path.line_to(c2 + unit(theta - phi) * r2);
    arc_to(&mut path, c2, r2, theta - phi, 2.0 * phi);
    path.line_to(c1 + unit(theta + phi) * r1);
    arc_to(&mut path, c1, r1, theta + phi, 2.0_f64.mul_add(-phi, TAU));
    path.close_path();
    path
}

/// The outline of a brush stroke through `samples` as one filled path (outer contours and holes), or `None` when
/// there is nothing to paint.
#[must_use]
pub fn brush_outline(samples: &[BrushSample]) -> Option<BezPath> {
    let first = samples.first()?;
    let mut pieces: Vec<PathData> = Vec::new();
    if samples.len() == 1 {
        pieces.push(PathData::from_bezpath(&Circle::new(first.pos, first.radius.max(0.05)).to_path(0.01)));
    }
    for pair in samples.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.last()) else { continue };
        pieces.push(PathData::from_bezpath(&capsule(a.pos, a.radius.max(0.05), b.pos, b.radius.max(0.05))));
    }
    let inputs: Vec<(&PathData, FillRule)> = pieces.iter().map(|p| (p, FillRule::NonZero)).collect();
    let united = unite_all(&inputs);
    let path = united.to_bezpath();
    (!path.elements().is_empty()).then_some(path)
}

/// Keeps a sample only when it moved far enough from the previous kept one.
#[must_use]
pub fn thin_samples(samples: &[BrushSample]) -> Vec<BrushSample> {
    let mut out: Vec<BrushSample> = Vec::with_capacity(samples.len());
    for (i, s) in samples.iter().enumerate() {
        let last = i.saturating_add(1) == samples.len();
        let keep = out.last().is_none_or(|prev| prev.pos.distance(s.pos) >= (s.radius * 0.25).max(0.75) || last);
        if keep {
            out.push(*s);
        }
    }
    out
}

/// Fits the pencil's polyline according to the mode. `zoom` converts screen tolerances to stage pixels.
#[must_use]
pub fn fit_polyline(points: &[Point], mode: PencilMode, zoom: f64, close: bool) -> BezPath {
    let mut poly = BezPath::new();
    if let Some((first, rest)) = points.split_first() {
        poly.move_to(*first);
        for p in rest {
            poly.line_to(*p);
        }
        if close {
            poly.close_path();
        }
    }
    let zoom = zoom.max(1.0e-6);
    let options = match mode {
        PencilMode::Straighten => SimplifyOptions { tolerance: 4.0 / zoom, corner_angle_deg: 20.0, straight_lines: true },
        PencilMode::Smooth => SimplifyOptions { tolerance: 2.5 / zoom, corner_angle_deg: 40.0, straight_lines: false },
        PencilMode::Ink => SimplifyOptions { tolerance: 0.75 / zoom, corner_angle_deg: 60.0, straight_lines: false },
    };
    simplify_with(&PathData::from_bezpath(&poly), &options).to_bezpath()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_covers_both_circles_and_the_middle() {
        let path = capsule(Point::new(0.0, 0.0), 5.0, Point::new(40.0, 0.0), 10.0);
        assert!(path.winding(Point::new(0.0, 0.0)) != 0);
        assert!(path.winding(Point::new(40.0, 9.0)) != 0);
        assert!(path.winding(Point::new(20.0, 0.0)) != 0);
        assert!(path.winding(Point::new(20.0, 9.0)) == 0);
        assert!(path.winding(Point::new(-6.0, 0.0)) == 0);
    }

    #[test]
    fn nested_circles_collapse_to_the_bigger_one() {
        let path = capsule(Point::new(0.0, 0.0), 10.0, Point::new(1.0, 0.0), 2.0);
        assert!(path.winding(Point::new(-9.0, 0.0)) != 0);
    }

    #[test]
    fn brush_outline_of_a_dab_is_a_disc() {
        let path = brush_outline(&[BrushSample { pos: Point::new(10.0, 10.0), radius: 4.0 }]).expect("outline");
        assert!(path.winding(Point::new(10.0, 10.0)) != 0);
        assert!(path.winding(Point::new(15.0, 10.0)) == 0);
    }

    #[test]
    fn brush_outline_follows_the_width_changes() {
        let samples = [
            BrushSample { pos: Point::new(0.0, 0.0), radius: 1.0 },
            BrushSample { pos: Point::new(30.0, 0.0), radius: 6.0 },
            BrushSample { pos: Point::new(60.0, 0.0), radius: 1.0 },
        ];
        let path = brush_outline(&samples).expect("outline");
        assert!(path.winding(Point::new(30.0, 5.0)) != 0);
        assert!(path.winding(Point::new(2.0, 5.0)) == 0);
    }

    #[test]
    fn smoothing_reduces_a_dense_polyline() {
        let points: Vec<Point> = (0..60).map(|i| Point::new(f64::from(i), (f64::from(i) * 0.2).sin() * 10.0)).collect();
        let path = fit_polyline(&points, PencilMode::Smooth, 1.0, false);
        assert!(path.elements().len() < points.len() / 2);
        let straight = fit_polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.1), Point::new(20.0, 0.0)], PencilMode::Straighten, 1.0, false);
        assert_eq!(straight.elements().len(), 2);
    }
}
