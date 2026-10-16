//! Variable-width strokes: width profiles, and baking a stroke into a filled outline that follows one.
//!
//! The model stores strokes with one width. A width profile (the Width tool's points, or one of the presets of the
//! Properties panel) turns the stroke into the filled outline of a line whose width changes along its length, which
//! draws the same everywhere (stage, SWF, SVG, video) because it is an ordinary fill. The outline is the union of
//! round-capped segments, so joins and closed outlines need no special cases.

#![allow(clippy::many_single_char_names, reason = "segment geometry reads best with the usual a, b, p, t, u names")]

use anim_core::kurbo::{self, PathEl};
use anim_core::{BezPath, Point};
use anim_shape::{EdgeShape, split_subpaths};
use serde::{Deserialize, Serialize};

use crate::freehand::{BrushSample, brush_outline};
use crate::shapeops::{Parts, decompose, rebuild};

/// Flattening tolerance when a stroke is walked, in pixels.
const FLATTEN_TOLERANCE: f64 = 0.1;
/// Most samples along one stroke.
const MAX_SAMPLES: f64 = 600.0;
/// Narrowest the outline is allowed to get, as a fraction of the stroke width (a profile never closes a line).
const MIN_FRACTION: f64 = 0.02;

/// The width of the line at one place along it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WidthPoint {
    /// Where along the line, 0 at its start to 1 at its end.
    pub at: f64,
    /// The width there, in pixels.
    pub width: f64,
}

/// How the width of a line changes along it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidthProfile {
    /// The stroke width everywhere.
    #[default]
    Uniform,
    /// Pointed at both ends, widest in the middle.
    TaperBoth,
    /// Pointed at the start.
    TaperStart,
    /// Pointed at the end.
    TaperEnd,
    /// Thin ends that swell sharply in the middle.
    Swell,
    /// Wide ends pinched in the middle.
    Pinch,
    /// Widths at chosen places, blended smoothly in between.
    Points(Vec<WidthPoint>),
}

impl WidthProfile {
    /// The presets in the order the Properties panel lists them.
    pub const PRESETS: [Self; 6] = [Self::Uniform, Self::TaperBoth, Self::TaperStart, Self::TaperEnd, Self::Swell, Self::Pinch];

    /// The name in the Properties panel.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Uniform => "Uniform",
            Self::TaperBoth => "Width Profile 1",
            Self::TaperStart => "Width Profile 2",
            Self::TaperEnd => "Width Profile 3",
            Self::Swell => "Width Profile 4",
            Self::Pinch => "Width Profile 5",
            Self::Points(_) => "Custom",
        }
    }

    /// The width at `t` (0 to 1 along the line) for a stroke of width `base`.
    #[must_use]
    pub fn width_at(&self, t: f64, base: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        let factor = match self {
            Self::Uniform => 1.0,
            Self::TaperBoth => (std::f64::consts::PI * t).sin(),
            Self::TaperStart => t,
            Self::TaperEnd => 1.0 - t,
            Self::Swell => (std::f64::consts::PI * t).sin().powi(3),
            Self::Pinch => 0.25_f64.mul_add(-(std::f64::consts::PI * t).sin(), 1.0).max(0.0).powi(2).max(0.2),
            Self::Points(points) => return Self::point_width(points, t, base),
        };
        (base * factor).max(base * MIN_FRACTION)
    }

    fn point_width(points: &[WidthPoint], t: f64, base: f64) -> f64 {
        let mut sorted: Vec<WidthPoint> = points.iter().copied().filter(|p| p.at.is_finite() && p.width.is_finite()).collect();
        sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
        let (Some(first), Some(last)) = (sorted.first(), sorted.last()) else { return base };
        let width = if t <= first.at {
            first.width
        } else if t >= last.at {
            last.width
        } else {
            sorted.windows(2).find(|w| w.first().zip(w.last()).is_some_and(|(a, b)| t >= a.at && t <= b.at)).and_then(|w| w.first().zip(w.last())).map_or(
                base,
                |(a, b)| {
                    let span = (b.at - a.at).max(1.0e-9);
                    let u = (t - a.at) / span;
                    // Smooth blend: no kinks where two points meet.
                    let eased = u * u * 2.0_f64.mul_add(-u, 3.0);
                    (b.width - a.width).mul_add(eased, a.width)
                },
            )
        };
        width.max(base * MIN_FRACTION)
    }

    /// This profile with a point added (a point within 2% of an existing one replaces it). Presets become points
    /// sampled from the preset first, so adding one point to "Width Profile 1" keeps its shape elsewhere.
    #[must_use]
    pub fn with_point(&self, at: f64, width: f64, base: f64) -> Self {
        let mut points = match self {
            Self::Points(points) => points.clone(),
            Self::Uniform => vec![WidthPoint { at: 0.0, width: base }, WidthPoint { at: 1.0, width: base }],
            preset => (0..=8).map(|i| WidthPoint { at: f64::from(i) / 8.0, width: preset.width_at(f64::from(i) / 8.0, base) }).collect(),
        };
        points.retain(|p| (p.at - at).abs() > 0.02);
        points.push(WidthPoint { at: at.clamp(0.0, 1.0), width: width.max(base * MIN_FRACTION) });
        points.sort_by(|a, b| a.at.total_cmp(&b.at));
        Self::Points(points)
    }
}

/// Where a stroke was hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeHit {
    /// Index into the shape's stroke runs (see [`strokes_of`]).
    pub stroke: usize,
    /// Index of the sub-path inside that stroke run.
    pub subpath: usize,
    /// How far along the sub-path, 0 to 1.
    pub t: f64,
    /// The nearest point on the stroke.
    pub point: Point,
    /// How far the pointer is from it.
    pub distance: f64,
}

/// One walked sub-path: its points and where each lies along it.
#[derive(Debug, Clone)]
struct Walk {
    points: Vec<Point>,
    /// Distance along the path at each point.
    along: Vec<f64>,
}

impl Walk {
    fn length(&self) -> f64 {
        self.along.last().copied().unwrap_or(0.0)
    }
}

fn walk(path: &BezPath) -> Walk {
    let mut points: Vec<Point> = Vec::new();
    let mut start = Point::ZERO;
    kurbo::flatten(path.iter(), FLATTEN_TOLERANCE, |el| match el {
        PathEl::MoveTo(p) => {
            start = p;
            points.push(p);
        }
        PathEl::LineTo(p) => points.push(p),
        PathEl::ClosePath => points.push(start),
        _ => {}
    });
    points.dedup_by(|a, b| a.distance(*b) < 1.0e-9);
    let mut along = Vec::with_capacity(points.len());
    let mut total = 0.0;
    for (i, p) in points.iter().enumerate() {
        if let Some(prev) = i.checked_sub(1).and_then(|j| points.get(j)) {
            total += prev.distance(*p);
        }
        along.push(total);
    }
    Walk { points, along }
}

/// A place along a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathSample {
    pub pos: Point,
    /// How far along the path, 0 to 1.
    pub t: f64,
    /// Distance from the start of the path.
    pub along: f64,
    /// Unit direction of travel (zero for a path without length).
    pub tangent: anim_core::Vec2,
}

/// Evenly spaced places along the first sub-path of `path`, `spacing` apart (the ends are always included).
#[must_use]
pub fn sample_path(path: &BezPath, spacing: f64) -> Vec<PathSample> {
    let w = walk(path);
    let length = w.length();
    if w.points.len() < 2 || length <= 0.0 || !spacing.is_finite() || spacing <= 0.0 {
        return Vec::new();
    }
    let count = anim_core::num::round_f64_to_u32((length / spacing).ceil().clamp(1.0, MAX_SAMPLES * 4.0));
    let mut out = Vec::new();
    let mut segment = 0_usize;
    for k in 0..=count {
        let t = f64::from(k) / f64::from(count);
        let s = t * length;
        while w.along.get(segment.saturating_add(1)).is_some_and(|next| *next < s) && segment.saturating_add(2) < w.points.len() {
            segment = segment.saturating_add(1);
        }
        let (Some(&a), Some(&b)) = (w.points.get(segment), w.points.get(segment.saturating_add(1))) else { continue };
        let (sa, sb) = (w.along.get(segment).copied().unwrap_or(0.0), w.along.get(segment.saturating_add(1)).copied().unwrap_or(length));
        let u = if sb > sa { ((s - sa) / (sb - sa)).clamp(0.0, 1.0) } else { 0.0 };
        let d = b - a;
        let len = d.hypot();
        let tangent = if len > 0.0 { d / len } else { anim_core::Vec2::ZERO };
        out.push(PathSample { pos: a.lerp(b, u), t, along: s, tangent });
    }
    out
}

/// The strokes of a shape as `(base width, walks)` per stroke run, in the order [`decompose`] lists them.
fn strokes_of(parts: &Parts) -> Vec<Vec<(BezPath, Walk)>> {
    parts.strokes.iter().map(|(_, path)| split_subpaths(path).into_iter().map(|sub| (sub.clone(), walk(&sub))).collect()).collect()
}

/// The stroke of `shape` nearest to `p` (the shape's pixels) within `tolerance`.
#[must_use]
pub fn stroke_hit(shape: &EdgeShape, p: Point, tolerance: f64) -> Option<StrokeHit> {
    let parts = decompose(shape);
    let mut best: Option<StrokeHit> = None;
    for (si, subs) in strokes_of(&parts).iter().enumerate() {
        let half = parts.strokes.get(si).map_or(0.0, |(s, _)| s.solid().weight / 2.0);
        for (ui, (_, w)) in subs.iter().enumerate() {
            let length = w.length();
            for (i, pair) in w.points.windows(2).enumerate() {
                let (Some(&a), Some(&b)) = (pair.first(), pair.last()) else { continue };
                let ab = b - a;
                let len2 = ab.hypot2();
                let u = if len2 <= 1.0e-12 { 0.0 } else { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) };
                let q = a + ab * u;
                let d = q.distance(p);
                if d <= tolerance + half && best.is_none_or(|b| d < b.distance) {
                    let before = w.along.get(i).copied().unwrap_or(0.0);
                    let s = u.mul_add(len2.sqrt(), before);
                    let t = if length > 0.0 { (s / length).clamp(0.0, 1.0) } else { 0.0 };
                    best = Some(StrokeHit { stroke: si, subpath: ui, t, point: q, distance: d });
                }
            }
        }
    }
    best
}

/// The base width and line of the stroke run `stroke`, or `None` when the shape has fewer.
#[must_use]
pub fn base_width(shape: &EdgeShape, stroke: usize) -> Option<f64> {
    let parts = decompose(shape);
    parts.strokes.get(stroke).map(|(s, _)| s.solid().weight.max(1.0))
}

/// The filled outline of one walked sub-path with the profile applied.
fn outline(w: &Walk, profile: &WidthProfile, base: f64) -> Option<BezPath> {
    let length = w.length();
    if w.points.len() < 2 || length <= 0.0 {
        return None;
    }
    let count = length.clamp(2.0, MAX_SAMPLES);
    let n = anim_core::num::round_f64_to_u32(count);
    let mut samples: Vec<BrushSample> = Vec::new();
    let mut segment = 0_usize;
    for k in 0..=n {
        let t = f64::from(k) / f64::from(n);
        let s = t * length;
        while w.along.get(segment.saturating_add(1)).is_some_and(|next| *next < s) && segment.saturating_add(2) < w.points.len() {
            segment = segment.saturating_add(1);
        }
        let (Some(&a), Some(&b)) = (w.points.get(segment), w.points.get(segment.saturating_add(1))) else { continue };
        let (sa, sb) = (w.along.get(segment).copied().unwrap_or(0.0), w.along.get(segment.saturating_add(1)).copied().unwrap_or(length));
        let u = if sb > sa { ((s - sa) / (sb - sa)).clamp(0.0, 1.0) } else { 0.0 };
        samples.push(BrushSample { pos: a.lerp(b, u), radius: profile.width_at(t, base) / 2.0 });
    }
    brush_outline(&samples)
}

/// Turns the stroke run `stroke` (every run when `None`) of `shape` into fills.
///
/// The sub-path `subpath` (every sub-path when `None`) gets `profile`; the others the plain stroke width. `None` when
/// there is no such stroke.
#[must_use]
pub fn bake(shape: &EdgeShape, stroke: Option<usize>, subpath: Option<usize>, profile: &WidthProfile) -> Option<EdgeShape> {
    let parts = decompose(shape);
    let walks = strokes_of(&parts);
    let mut out = Parts { fills: parts.fills.clone(), strokes: Vec::new() };
    let mut changed = false;
    for (si, ((style, path), subs)) in parts.strokes.iter().zip(&walks).enumerate() {
        if stroke.is_some_and(|only| only != si) {
            out.strokes.push((style.clone(), path.clone()));
            continue;
        }
        let base = style.solid().weight.max(1.0);
        let paint = style.solid().fill.clone();
        for (ui, (_, w)) in subs.iter().enumerate() {
            let chosen = if subpath.is_none_or(|only| only == ui) { profile } else { &WidthProfile::Uniform };
            if let Some(shape_path) = outline(w, chosen, base) {
                out.fills.push((paint.clone(), shape_path));
                changed = true;
            }
        }
    }
    changed.then(|| rebuild(&out)).filter(|s| !s.is_empty())
}

/// The shape with the profile applied to every stroke, or `None` when it has no stroke.
#[must_use]
pub fn apply_profile(shape: &EdgeShape, profile: &WidthProfile) -> Option<EdgeShape> {
    bake(shape, None, None, profile)
}

#[cfg(test)]
mod tests {
    use anim_core::Rgba8;
    use anim_shape::{FillStyle, PathInput, StrokeStyle};

    use super::*;

    fn line(width: f64) -> EdgeShape {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        EdgeShape::from_paths(&[PathInput::new(p, None, Some(StrokeStyle::plain(width, Rgba8::BLACK)))])
    }

    #[test]
    fn presets_taper_where_they_say() {
        let base = 10.0;
        assert!(WidthProfile::TaperBoth.width_at(0.0, base) < 0.5);
        assert!((WidthProfile::TaperBoth.width_at(0.5, base) - 10.0).abs() < 1.0e-9);
        assert!(WidthProfile::TaperStart.width_at(0.1, base) < WidthProfile::TaperStart.width_at(0.9, base));
        assert!(WidthProfile::TaperEnd.width_at(0.1, base) > WidthProfile::TaperEnd.width_at(0.9, base));
        assert!((WidthProfile::Uniform.width_at(0.3, base) - 10.0).abs() < 1.0e-9);
    }

    #[test]
    fn points_blend_smoothly_and_hold_at_the_ends() {
        let profile = WidthProfile::Uniform.with_point(0.5, 30.0, 10.0).with_point(0.0, 4.0, 10.0);
        assert!((profile.width_at(0.0, 10.0) - 4.0).abs() < 1.0e-9);
        assert!((profile.width_at(0.5, 10.0) - 30.0).abs() < 1.0e-9);
        assert!((profile.width_at(1.0, 10.0) - 10.0).abs() < 1.0e-9, "the far end keeps the stroke width");
        let mid = profile.width_at(0.25, 10.0);
        assert!(mid > 4.0 && mid < 30.0);
        // A point near an existing one replaces it.
        let WidthProfile::Points(points) = profile.with_point(0.51, 12.0, 10.0) else { panic!("not points") };
        assert_eq!(points.len(), 3);
        assert!(points.iter().any(|p| (p.width - 12.0).abs() < 1.0e-9) && !points.iter().any(|p| (p.width - 30.0).abs() < 1.0e-9));
    }

    #[test]
    fn baking_a_taper_makes_a_pointed_fill_and_removes_the_stroke() {
        let baked = apply_profile(&line(10.0), &WidthProfile::TaperBoth).expect("baked");
        assert!(baked.strokes.is_empty(), "the stroke became a fill");
        assert!(baked.fill_at(Point::new(50.0, 3.0)).is_some(), "the middle is fat");
        assert!(baked.fill_at(Point::new(2.0, 3.0)).is_none(), "the end is pointed");
        let b = baked.bounds().unwrap();
        assert!(b.height() > 8.0 && b.height() <= 10.5, "{b:?}");
    }

    #[test]
    fn the_hit_reports_how_far_along_the_stroke_it_is() {
        let hit = stroke_hit(&line(4.0), Point::new(25.0, 1.0), 3.0).expect("hit");
        assert!((hit.t - 0.25).abs() < 0.01, "{hit:?}");
        assert!((hit.point.x - 25.0).abs() < 0.01 && hit.point.y.abs() < 0.01);
        assert!(stroke_hit(&line(4.0), Point::new(25.0, 20.0), 3.0).is_none());
    }

    #[test]
    fn a_shape_without_strokes_cannot_be_baked() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((10.0, 0.0));
        p.line_to((10.0, 10.0));
        p.close_path();
        let shape = EdgeShape::from_paths(&[PathInput::new(p, Some(FillStyle::Solid(Rgba8::BLACK)), None)]);
        assert!(apply_profile(&shape, &WidthProfile::TaperBoth).is_none());
    }

    #[test]
    fn only_the_chosen_subpath_gets_the_profile() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p.move_to((0.0, 50.0));
        p.line_to((100.0, 50.0));
        let shape = EdgeShape::from_paths(&[PathInput::new(p, None, Some(StrokeStyle::plain(10.0, Rgba8::BLACK)))]);
        let hit = stroke_hit(&shape, Point::new(50.0, 0.0), 2.0).unwrap();
        let baked = bake(&shape, Some(hit.stroke), Some(hit.subpath), &WidthProfile::TaperBoth).unwrap();
        let near_end = |y: f64| baked.fill_at(Point::new(3.0, y)).is_some();
        assert!(!near_end(2.5), "the chosen line is pointed at its end");
        assert!(near_end(52.5), "the other line keeps its width");
    }
}
