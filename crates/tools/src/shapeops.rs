//! Geometry operations on edge shapes: merge-mode drawing, paint bucket, ink bottle and eraser.
//!
//! Each operation splits a shape into its fill regions and stroke polylines (`EdgeShape::assemble`), edits those
//! with the `VectorCraft` boolean and planar-face operations, and builds a fresh shape with
//! `EdgeShape::from_paths`.
//! The result looks the same everywhere the operation did not touch, but the edges of a shape that went through
//! one are regenerated, so edge-level details of the original file (unedited XFL text) are not kept for it.

use anim_core::kurbo::{self, Line, ParamCurveNearest, PathEl, Shape as _};
use anim_core::num::{ceil_f64_to_i32, usize_to_f64};
use anim_core::{BezPath, Point, Rect};
use anim_shape::{EdgeShape, FillStyle, Hit, PathInput, StrokeStyle, split_subpaths};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_pathops::{BoolOp, Shape as VShape, area, boolean, live_paint, unite_all};

use crate::freehand::{BrushSample, brush_outline_shaped};
use crate::settings::{BrushMode, TipShape};

/// Flattening tolerance when a stroke has to be cut, in pixels (one twip).
const FLATTEN_TOLERANCE: f64 = 0.05;
/// Length of the pieces a stroke is cut into when deciding what to keep, in pixels.
const CUT_STEP: f64 = 0.5;
/// Open stroke ends considered when closing gaps.
const MAX_GAP_ENDPOINTS: usize = 400;

/// A shape split into painted regions and stroked polylines, each with its style.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parts {
    /// Filled regions: all contours of the region in one path, to be filled with the even-odd rule.
    pub fills: Vec<(FillStyle, BezPath)>,
    /// Stroked polylines (open and closed) in one path per stroke style.
    pub strokes: Vec<(StrokeStyle, BezPath)>,
}

/// Splits a shape into regions and polylines.
#[must_use]
pub fn decompose(shape: &EdgeShape) -> Parts {
    let assembled = shape.assemble();
    Parts {
        fills: assembled.fills.into_iter().filter_map(|r| shape.fill(r.style).map(|f| (f.clone(), r.path))).collect(),
        strokes: assembled.strokes.into_iter().filter_map(|r| shape.stroke(r.style).map(|s| (s.clone(), r.path))).collect(),
    }
}

/// Builds a shape from regions and polylines. Regions of equal style are united.
#[must_use]
pub fn rebuild(parts: &Parts) -> EdgeShape {
    let mut styles: Vec<&FillStyle> = Vec::new();
    let mut groups: Vec<Vec<PathData>> = Vec::new();
    for (style, path) in &parts.fills {
        let data = PathData::from_bezpath(path);
        if let Some(i) = styles.iter().position(|s| *s == style) {
            if let Some(group) = groups.get_mut(i) {
                group.push(data);
            }
        } else {
            styles.push(style);
            groups.push(vec![data]);
        }
    }
    let mut inputs: Vec<PathInput> = Vec::new();
    for (style, group) in styles.into_iter().zip(groups) {
        let path = if let [single] = group.as_slice() {
            single.to_bezpath()
        } else {
            let refs: Vec<(&PathData, FillRule)> = group.iter().map(|p| (p, FillRule::EvenOdd)).collect();
            unite_all(&refs).to_bezpath()
        };
        if !path.elements().is_empty() {
            inputs.push(PathInput::new(path, Some(style.clone()), None));
        }
    }
    let mut stroke_styles: Vec<&StrokeStyle> = Vec::new();
    let mut stroke_paths: Vec<BezPath> = Vec::new();
    for (style, path) in &parts.strokes {
        if let Some(i) = stroke_styles.iter().position(|s| *s == style) {
            if let Some(existing) = stroke_paths.get_mut(i) {
                existing.extend(path.iter());
            }
        } else {
            stroke_styles.push(style);
            stroke_paths.push(path.clone());
        }
    }
    for (style, path) in stroke_styles.into_iter().zip(stroke_paths) {
        inputs.push(PathInput::new(path, None, Some(style.clone())));
    }
    EdgeShape::from_paths(&inputs)
}

fn path_bounds(path: &BezPath) -> Option<Rect> {
    let r = path.bounding_box();
    (r.x0.is_finite() && r.x1.is_finite() && r.y0.is_finite() && r.y1.is_finite() && !path.elements().is_empty()).then_some(r)
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Distance from `p` to the nearest point of a path, or infinity for an empty path.
fn distance_to_path(path: &BezPath, p: Point) -> f64 {
    path.segments().map(|s| ParamCurveNearest::nearest(&s, p, 1.0e-3).distance_sq.sqrt()).fold(f64::INFINITY, f64::min)
}

fn distance_to_polyline(points: &[Point], p: Point) -> f64 {
    match points {
        [] => f64::INFINITY,
        [only] => only.distance(p),
        _ => points
            .windows(2)
            .filter_map(|w| Some(Line::new(*w.first()?, *w.last()?)))
            .map(|l| l.nearest(p, 1.0e-3).distance_sq.sqrt())
            .fold(f64::INFINITY, f64::min),
    }
}

/// Cuts `path` into pieces and keeps those whose midpoint satisfies `keep`. Subpaths outside `region` (when given)
/// or with nothing to cut are kept as they are, curves included; cut subpaths become polylines.
#[must_use]
pub fn trim_path(path: &BezPath, region: Option<Rect>, keep: &dyn Fn(Point) -> bool) -> BezPath {
    let mut out = BezPath::new();
    for sub in split_subpaths(path) {
        if region.is_some_and(|r| path_bounds(&sub).is_none_or(|b| !overlaps(b, r))) {
            out.extend(sub.iter());
            continue;
        }
        let mut polyline: Vec<PathEl> = Vec::new();
        kurbo::flatten(sub.iter(), FLATTEN_TOLERANCE, |el| polyline.push(el));
        let mut points: Vec<Point> = Vec::new();
        let mut start = Point::ZERO;
        for el in &polyline {
            match *el {
                PathEl::MoveTo(p) => {
                    start = p;
                    points.push(p);
                }
                PathEl::LineTo(p) => points.push(p),
                PathEl::ClosePath => points.push(start),
                _ => {}
            }
        }
        let mut cut = BezPath::new();
        let mut removed = false;
        let mut open = false;
        for pair in points.windows(2) {
            let (Some(&a), Some(&b)) = (pair.first(), pair.last()) else { continue };
            let pieces = usize::try_from(ceil_f64_to_i32(a.distance(b) / CUT_STEP).max(1)).unwrap_or(1);
            for k in 0..pieces {
                let ta = usize_to_f64(k) / usize_to_f64(pieces);
                let tb = usize_to_f64(k.saturating_add(1)) / usize_to_f64(pieces);
                let (pa, pb) = (a.lerp(b, ta), a.lerp(b, tb));
                if keep(pa.midpoint(pb)) {
                    if !open {
                        cut.move_to(pa);
                        open = true;
                    }
                    cut.line_to(pb);
                } else {
                    removed = true;
                    open = false;
                }
            }
        }
        if removed {
            out.extend(cut.iter());
        } else {
            out.extend(sub.iter());
        }
    }
    out
}

fn gap_bridges(strokes: &[(StrokeStyle, BezPath)], gap: f64) -> Vec<BezPath> {
    if gap <= 0.0 {
        return Vec::new();
    }
    let mut ends: Vec<Point> = Vec::new();
    for (_, path) in strokes {
        for sub in split_subpaths(path) {
            let closed = sub.elements().iter().any(|e| matches!(e, PathEl::ClosePath));
            let (Some(first), Some(last)) = (sub.elements().first(), sub.elements().last()) else { continue };
            let start = match first {
                PathEl::MoveTo(p) => *p,
                _ => continue,
            };
            let end = match last {
                PathEl::LineTo(p) | PathEl::MoveTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => *p,
                PathEl::ClosePath => start,
            };
            if !closed && start.distance(end) > 1.0e-6 && ends.len() < MAX_GAP_ENDPOINTS {
                ends.push(start);
                ends.push(end);
            }
        }
    }
    let mut out = Vec::new();
    for (i, a) in ends.iter().enumerate() {
        for b in ends.iter().skip(i.saturating_add(1)) {
            let d = a.distance(*b);
            if d > 1.0e-6 && d <= gap {
                let mut bridge = BezPath::new();
                bridge.move_to(*a);
                bridge.line_to(*b);
                out.push(bridge);
            }
        }
    }
    out
}

/// The bounded area around `p` that the fills and strokes of `parts` enclose (outer contour and holes), closing
/// gaps in strokes up to `gap` pixels.
#[must_use]
pub fn face_at(parts: &Parts, p: Point, gap: f64) -> Option<BezPath> {
    let mut shapes: Vec<VShape> = Vec::new();
    let mut key = 0_u64;
    for (_, path) in &parts.fills {
        shapes.push(VShape::new(PathData::from_bezpath(path), FillRule::EvenOdd, key));
        key = key.saturating_add(1);
    }
    for (_, path) in &parts.strokes {
        shapes.push(VShape::new(PathData::from_bezpath(path), FillRule::EvenOdd, key));
        key = key.saturating_add(1);
    }
    for bridge in gap_bridges(&parts.strokes, gap) {
        shapes.push(VShape::new(PathData::from_bezpath(&bridge), FillRule::EvenOdd, key));
        key = key.saturating_add(1);
    }
    if shapes.is_empty() {
        return None;
    }
    let (faces, _) = live_paint(&shapes);
    faces
        .into_iter()
        .filter(|f| f.contains(p))
        .min_by(|a, b| area(&a.path, FillRule::NonZero).total_cmp(&area(&b.path, FillRule::NonZero)))
        .map(|f| f.path.to_bezpath())
}

/// Fills the area around `p` with `fill`. The area is the bounded region enclosed by the shape's fills and
/// strokes; fills underneath are cut so the new fill replaces them. `None` when `p` is in no bounded area.
#[must_use]
pub fn paint_bucket(shape: &EdgeShape, p: Point, fill: &FillStyle, gap: f64, lock: bool) -> Option<EdgeShape> {
    let parts = decompose(shape);
    let face = face_at(&parts, p, gap)?;
    let face_data = PathData::from_bezpath(&face);
    let face_bounds = path_bounds(&face);
    // Without lock fill a gradient or bitmap is placed on the area that is painted.
    let placed;
    let fill = match face_bounds {
        Some(bounds) if !lock => {
            placed = crate::fillfit::fit_fill(fill, bounds);
            &placed
        }
        _ => fill,
    };
    let mut out = Parts { fills: Vec::new(), strokes: parts.strokes.clone() };
    for (style, path) in &parts.fills {
        let touches = face_bounds.zip(path_bounds(path)).is_some_and(|(a, b)| overlaps(a, b));
        if touches {
            let rest = boolean(&PathData::from_bezpath(path), FillRule::EvenOdd, &face_data, FillRule::NonZero, BoolOp::Difference);
            if !rest.is_empty() {
                out.fills.push((style.clone(), rest.to_bezpath()));
            }
        } else {
            out.fills.push((style.clone(), path.clone()));
        }
    }
    out.fills.push((fill.clone(), face));
    Some(rebuild(&out))
}

/// Applies `stroke` to the stroke under `p` (within `tolerance`), or outlines the area around `p` with it.
#[must_use]
pub fn ink_bottle(shape: &EdgeShape, p: Point, tolerance: f64, stroke: &StrokeStyle) -> Option<EdgeShape> {
    if let Some(Hit::Stroke { .. }) = shape.hit_test(p, tolerance) {
        let mut best: Option<(usize, f64)> = None;
        for (i, edge) in shape.edges.iter().enumerate().filter(|(_, e)| e.stroke_style != 0) {
            for segment in &edge.segments {
                let d = ParamCurveNearest::nearest(&segment.to_path_seg(1.0 / anim_shape::TWIPS_PER_PX), p, 1.0e-3).distance_sq.sqrt();
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
        }
        let (index, _) = best?;
        let mut out = shape.clone();
        let style = out.intern_stroke(stroke);
        let edge = out.edges.get_mut(index)?;
        if edge.stroke_style == style {
            return None;
        }
        edge.stroke_style = style;
        edge.forget_original();
        return Some(out);
    }
    let parts = decompose(shape);
    shape.fill_at(p)?;
    let face = face_at(&parts, p, 0.0)?;
    let region = path_bounds(&face);
    let mut out = Parts { fills: parts.fills.clone(), strokes: Vec::new() };
    for (style, path) in &parts.strokes {
        let trimmed = trim_path(path, region.map(|r| r.inflate(1.0, 1.0)), &|pt| distance_to_path(&face, pt) > 0.6);
        if !trimmed.elements().is_empty() {
            out.strokes.push((style.clone(), trimmed));
        }
    }
    out.strokes.push((stroke.clone(), face));
    Some(rebuild(&out))
}

/// Paints `outline` (in the shape's own coordinates) into `existing` with one of the brush modes other than
/// paint normal. Strokes are never cut. `None` when the mode leaves nothing to paint.
#[must_use]
pub fn paint_clipped(existing: &EdgeShape, outline: &BezPath, fill: &FillStyle, mode: BrushMode, start: Point) -> Option<EdgeShape> {
    let parts = decompose(existing);
    let brush = PathData::from_bezpath(outline);
    let union_of_fills = || {
        let datas: Vec<PathData> = parts.fills.iter().map(|(_, p)| PathData::from_bezpath(p)).collect();
        let refs: Vec<(&PathData, FillRule)> = datas.iter().map(|p| (p, FillRule::EvenOdd)).collect();
        unite_all(&refs)
    };
    let clip = match mode {
        BrushMode::PaintNormal | BrushMode::PaintFills => brush,
        BrushMode::PaintBehind => boolean(&brush, FillRule::NonZero, &union_of_fills(), FillRule::NonZero, BoolOp::Difference),
        BrushMode::PaintSelection => boolean(&brush, FillRule::NonZero, &union_of_fills(), FillRule::NonZero, BoolOp::Intersect),
        BrushMode::PaintInside => {
            let face = face_at(&parts, start, 0.0)?;
            boolean(&brush, FillRule::NonZero, &PathData::from_bezpath(&face), FillRule::NonZero, BoolOp::Intersect)
        }
    };
    if clip.is_empty() {
        return None;
    }
    let clip_path = clip.to_bezpath();
    let bounds = path_bounds(&clip_path)?;
    let mut out = Parts { fills: Vec::new(), strokes: parts.strokes.clone() };
    for (style, path) in &parts.fills {
        let cuts = mode != BrushMode::PaintBehind && path_bounds(path).is_some_and(|b| overlaps(b, bounds));
        if cuts {
            let rest = boolean(&PathData::from_bezpath(path), FillRule::EvenOdd, &clip, FillRule::NonZero, BoolOp::Difference);
            if !rest.is_empty() {
                out.fills.push((style.clone(), rest.to_bezpath()));
            }
        } else {
            out.fills.push((style.clone(), path.clone()));
        }
    }
    out.fills.push((fill.clone(), clip_path));
    Some(rebuild(&out))
}

/// The eraser faucet: removes the whole stroke segment under `p`, or else the whole area of the fill under `p`.
#[must_use]
pub fn faucet(shape: &EdgeShape, p: Point, tolerance: f64, fills: bool, lines: bool) -> Erased {
    if lines && let Some(Hit::Stroke { .. }) = shape.hit_test(p, tolerance) {
        let mut best: Option<(usize, f64)> = None;
        for (i, edge) in shape.edges.iter().enumerate().filter(|(_, e)| e.stroke_style != 0) {
            for segment in &edge.segments {
                let d = ParamCurveNearest::nearest(&segment.to_path_seg(1.0 / anim_shape::TWIPS_PER_PX), p, 1.0e-3).distance_sq.sqrt();
                if best.is_none_or(|(_, bd)| d < bd) {
                    best = Some((i, d));
                }
            }
        }
        let Some((index, _)) = best else { return Erased::Unchanged };
        let mut out = shape.clone();
        if let Some(edge) = out.edges.get_mut(index) {
            edge.stroke_style = 0;
            edge.forget_original();
        }
        out.edges.retain(|e| e.stroke_style != 0 || e.fill_style0 != 0 || e.fill_style1 != 0);
        return if out.is_empty() { Erased::Gone } else { Erased::Changed(out) };
    }
    if !fills || shape.fill_at(p).is_none() {
        return Erased::Unchanged;
    }
    let parts = decompose(shape);
    let Some(face) = face_at(&parts, p, 0.0) else { return Erased::Unchanged };
    let face_data = PathData::from_bezpath(&face);
    let mut out = Parts { fills: Vec::new(), strokes: parts.strokes.clone() };
    for (style, path) in &parts.fills {
        let rest = boolean(&PathData::from_bezpath(path), FillRule::EvenOdd, &face_data, FillRule::NonZero, BoolOp::Difference);
        if !rest.is_empty() {
            out.fills.push((style.clone(), rest.to_bezpath()));
        }
    }
    let rebuilt = rebuild(&out);
    if rebuilt.is_empty() { Erased::Gone } else { Erased::Changed(rebuilt) }
}

/// What the eraser removes from one shape.
#[derive(Debug, Clone, Copy)]
pub struct EraseSpec<'a> {
    /// The eraser path in the shape's own coordinates.
    pub points: &'a [Point],
    /// Eraser radius in the shape's own coordinates.
    pub radius: f64,
    /// The eraser tip.
    pub tip: TipShape,
    pub fills: bool,
    pub lines: bool,
    /// Only erase the fill that covers this point (the press position).
    pub only_fill_at: Option<Point>,
}

/// The result of erasing from a shape.
#[derive(Debug, Clone, PartialEq)]
pub enum Erased {
    Unchanged,
    Changed(EdgeShape),
    /// Nothing is left.
    Gone,
}

/// Erases along a brush path.
#[must_use]
pub fn erase(shape: &EdgeShape, spec: &EraseSpec<'_>) -> Erased {
    let Some(shape_bounds) = shape.bounds_with_strokes() else { return Erased::Unchanged };
    let (Some(first), true) = (spec.points.first(), spec.radius > 0.0) else { return Erased::Unchanged };
    let eraser_bounds = spec.points.iter().fold(Rect::from_points(*first, *first), |r, p| r.union_pt(*p)).inflate(spec.radius, spec.radius);
    if !overlaps(shape_bounds, eraser_bounds) {
        return Erased::Unchanged;
    }
    let samples: Vec<BrushSample> = spec.points.iter().map(|&pos| BrushSample { pos, radius: spec.radius }).collect();
    let Some(eraser) = brush_outline_shaped(&samples, spec.tip) else { return Erased::Unchanged };
    let eraser_data = PathData::from_bezpath(&eraser);
    let parts = decompose(shape);
    let only_style = spec.only_fill_at.and_then(|q| shape.fill_at(q));
    let mut out = Parts::default();
    let mut touched = false;
    for (style, path) in &parts.fills {
        let eligible = spec.fills && only_style.is_none_or(|s| shape.fill(s) == Some(style));
        if eligible && path_bounds(path).is_some_and(|b| overlaps(b, eraser_bounds)) {
            let rest = boolean(&PathData::from_bezpath(path), FillRule::EvenOdd, &eraser_data, FillRule::NonZero, BoolOp::Difference);
            touched = true;
            if !rest.is_empty() {
                out.fills.push((style.clone(), rest.to_bezpath()));
            }
        } else {
            out.fills.push((style.clone(), path.clone()));
        }
    }
    for (style, path) in &parts.strokes {
        if spec.lines && spec.only_fill_at.is_none() && path_bounds(path).is_some_and(|b| overlaps(b.inflate(1.0, 1.0), eraser_bounds)) {
            let trimmed = trim_path(path, Some(eraser_bounds), &|pt| distance_to_polyline(spec.points, pt) > spec.radius);
            if trimmed != *path {
                touched = true;
            }
            if !trimmed.elements().is_empty() {
                out.strokes.push((style.clone(), trimmed));
            }
        } else {
            out.strokes.push((style.clone(), path.clone()));
        }
    }
    if !touched {
        return Erased::Unchanged;
    }
    let rebuilt = rebuild(&out);
    if rebuilt.is_empty() { Erased::Gone } else { Erased::Changed(rebuilt) }
}

/// A shape cut in two.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Split {
    /// What lies inside the region (or matches the wand), if anything.
    pub inside: Option<EdgeShape>,
    /// Everything else, if anything.
    pub outside: Option<EdgeShape>,
}

fn non_empty(parts: &Parts) -> Option<EdgeShape> {
    let shape = rebuild(parts);
    (!shape.is_empty()).then_some(shape)
}

/// Cuts `shape` along the closed `region` (the Lasso tool): fills are intersected with it and subtracted from it,
/// strokes are cut at its border. Both are in the shape's own coordinates.
#[must_use]
pub fn split_by_region(shape: &EdgeShape, region: &BezPath) -> Split {
    let Some(region_bounds) = path_bounds(region) else { return Split { inside: None, outside: non_empty(&decompose(shape)) } };
    let parts = decompose(shape);
    let region_data = PathData::from_bezpath(region);
    let mut inside = Parts::default();
    let mut outside = Parts::default();
    for (style, path) in &parts.fills {
        if path_bounds(path).is_none_or(|b| !overlaps(b, region_bounds)) {
            outside.fills.push((style.clone(), path.clone()));
            continue;
        }
        let data = PathData::from_bezpath(path);
        let inner = boolean(&data, FillRule::EvenOdd, &region_data, FillRule::NonZero, BoolOp::Intersect);
        let rest = boolean(&data, FillRule::EvenOdd, &region_data, FillRule::NonZero, BoolOp::Difference);
        if !inner.is_empty() {
            inside.fills.push((style.clone(), inner.to_bezpath()));
        }
        if !rest.is_empty() {
            outside.fills.push((style.clone(), rest.to_bezpath()));
        }
    }
    for (style, path) in &parts.strokes {
        if path_bounds(path).is_none_or(|b| !overlaps(b.inflate(1.0, 1.0), region_bounds)) {
            outside.strokes.push((style.clone(), path.clone()));
            continue;
        }
        let inner = trim_path(path, Some(region_bounds), &|pt| region.winding(pt) != 0);
        let rest = trim_path(path, Some(region_bounds), &|pt| region.winding(pt) == 0);
        if !inner.elements().is_empty() {
            inside.strokes.push((style.clone(), inner));
        }
        if !rest.elements().is_empty() {
            outside.strokes.push((style.clone(), rest));
        }
    }
    Split { inside: non_empty(&inside), outside: non_empty(&outside) }
}

/// True when two fills are the same paint or two solid colours within `threshold` on every channel.
fn similar_fill(a: &FillStyle, b: &FillStyle, threshold: f64) -> bool {
    match (a, b) {
        (FillStyle::Solid(x), FillStyle::Solid(y)) => {
            [(x.r, y.r), (x.g, y.g), (x.b, y.b), (x.a, y.a)].into_iter().all(|(p, q)| f64::from(p.abs_diff(q)) <= threshold)
        }
        _ => a == b,
    }
}

/// The paint of the fill at `p` (the shape's pixels).
#[must_use]
pub fn paint_at(shape: &EdgeShape, p: Point) -> Option<FillStyle> {
    shape.fill(shape.fill_at(p)?).cloned()
}

/// The Lasso tool's magic wand: the fills of `shape` whose paint is similar to `clicked` are split from the rest,
/// strokes stay behind.
#[must_use]
pub fn split_by_paint(shape: &EdgeShape, clicked: &FillStyle, threshold: f64) -> Split {
    let parts = decompose(shape);
    let mut inside = Parts::default();
    let mut outside = Parts::default();
    for (style, path) in &parts.fills {
        if similar_fill(style, clicked, threshold) {
            inside.fills.push((style.clone(), path.clone()));
        } else {
            outside.fills.push((style.clone(), path.clone()));
        }
    }
    outside.strokes.clone_from(&parts.strokes);
    Split { inside: non_empty(&inside), outside: non_empty(&outside) }
}

/// Merge-mode drawing: `new` is drawn over `existing`. Its fills replace what lies underneath (fills are cut,
/// strokes under a fill are cut); its strokes lie on top. Regions of the same style are united.
#[must_use]
pub fn merge(existing: &EdgeShape, new: &EdgeShape) -> EdgeShape {
    if existing.is_empty() {
        return new.clone();
    }
    let old = decompose(existing);
    let added = decompose(new);
    let fill_inputs: Vec<PathData> = added.fills.iter().map(|(_, p)| PathData::from_bezpath(p)).collect();
    let cover = (!fill_inputs.is_empty()).then(|| {
        let refs: Vec<(&PathData, FillRule)> = fill_inputs.iter().map(|p| (p, FillRule::EvenOdd)).collect();
        unite_all(&refs)
    });
    let cover_bounds = cover.as_ref().and_then(|c| path_bounds(&c.to_bezpath()));
    let cover_path = cover.as_ref().map(PathData::to_bezpath);
    let mut out = Parts::default();
    for (style, path) in old.fills {
        match (&cover, cover_bounds) {
            (Some(cover), Some(bounds)) if path_bounds(&path).is_some_and(|b| overlaps(b, bounds)) => {
                let rest = boolean(&PathData::from_bezpath(&path), FillRule::EvenOdd, cover, FillRule::NonZero, BoolOp::Difference);
                if !rest.is_empty() {
                    out.fills.push((style, rest.to_bezpath()));
                }
            }
            _ => out.fills.push((style, path)),
        }
    }
    for (style, path) in old.strokes {
        match (&cover_path, cover_bounds) {
            (Some(cover), Some(bounds)) => {
                let trimmed = trim_path(&path, Some(bounds), &|pt| cover.winding(pt) == 0);
                if !trimmed.elements().is_empty() {
                    out.strokes.push((style, trimmed));
                }
            }
            _ => out.strokes.push((style, path)),
        }
    }
    out.fills.extend(added.fills);
    out.strokes.extend(added.strokes);
    rebuild(&out)
}

#[cfg(test)]
mod tests {
    use anim_core::Rgba8;
    use anim_core::kurbo::Rect as KRect;

    use super::*;

    fn rect_shape(x0: f64, y0: f64, x1: f64, y1: f64, fill: Option<Rgba8>, stroke: Option<f64>) -> EdgeShape {
        EdgeShape::from_paths(&[PathInput::new(
            KRect::new(x0, y0, x1, y1).to_path(0.1),
            fill.map(FillStyle::Solid),
            stroke.map(|w| StrokeStyle::plain(w, Rgba8::BLACK)),
        )])
    }

    fn red() -> Rgba8 {
        Rgba8::opaque(255, 0, 0)
    }

    fn blue() -> Rgba8 {
        Rgba8::opaque(0, 0, 255)
    }

    fn fill_color_at(shape: &EdgeShape, p: Point) -> Option<Rgba8> {
        shape.fill(shape.fill_at(p)?)?.solid_color()
    }

    #[test]
    fn decompose_and_rebuild_keep_what_is_painted() {
        let shape = rect_shape(0.0, 0.0, 100.0, 50.0, Some(red()), Some(2.0));
        let again = rebuild(&decompose(&shape));
        assert_eq!(fill_color_at(&again, Point::new(50.0, 25.0)), Some(red()));
        assert!(again.hit_test(Point::new(0.0, 25.0), 1.0).is_some());
        assert!(again.fill_at(Point::new(150.0, 25.0)).is_none());
    }

    #[test]
    fn merging_a_fill_over_a_fill_cuts_the_old_one() {
        let old = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), None);
        let new = rect_shape(50.0, 50.0, 150.0, 150.0, Some(blue()), None);
        let merged = merge(&old, &new);
        assert_eq!(fill_color_at(&merged, Point::new(25.0, 25.0)), Some(red()));
        assert_eq!(fill_color_at(&merged, Point::new(75.0, 75.0)), Some(blue()));
        assert_eq!(fill_color_at(&merged, Point::new(125.0, 125.0)), Some(blue()));
        assert!(merged.fill_at(Point::new(125.0, 25.0)).is_none());
    }

    #[test]
    fn merging_a_fill_cuts_strokes_underneath() {
        let old = rect_shape(0.0, 0.0, 100.0, 100.0, None, Some(2.0));
        let new = rect_shape(40.0, -20.0, 140.0, 50.0, Some(blue()), None);
        let merged = merge(&old, &new);
        // The top edge of the old rectangle (y = 0) passes through the new fill between x = 40 and 100.
        assert!(merged.hit_test(Point::new(20.0, 0.0), 1.0).is_some());
        assert!(!matches!(merged.hit_test(Point::new(70.0, 0.0), 1.0), Some(Hit::Stroke { .. })));
    }

    #[test]
    fn bucket_fills_an_area_enclosed_by_strokes() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, None, Some(2.0));
        let filled = paint_bucket(&shape, Point::new(50.0, 50.0), &FillStyle::Solid(blue()), 0.0, false).expect("enclosed");
        assert_eq!(fill_color_at(&filled, Point::new(50.0, 50.0)), Some(blue()));
        assert!(filled.hit_test(Point::new(0.0, 50.0), 1.0).is_some());
        assert!(paint_bucket(&shape, Point::new(300.0, 300.0), &FillStyle::Solid(blue()), 0.0, false).is_none());
    }

    #[test]
    fn bucket_recolours_only_the_clicked_face() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), Some(2.0));
        // A stroke across the rectangle splits it into two faces.
        let divider = EdgeShape::from_paths(&[PathInput::new(
            {
                let mut p = BezPath::new();
                p.move_to((50.0, -10.0));
                p.line_to((50.0, 110.0));
                p
            },
            None,
            Some(StrokeStyle::plain(2.0, Rgba8::BLACK)),
        )]);
        let both = merge(&shape, &divider);
        let painted = paint_bucket(&both, Point::new(25.0, 50.0), &FillStyle::Solid(blue()), 0.0, false).expect("face");
        assert_eq!(fill_color_at(&painted, Point::new(25.0, 50.0)), Some(blue()));
        assert_eq!(fill_color_at(&painted, Point::new(75.0, 50.0)), Some(red()));
    }

    #[test]
    fn bucket_closes_small_gaps() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((100.0, 0.0));
        p.line_to((100.0, 100.0));
        p.line_to((0.0, 100.0));
        p.line_to((0.0, 6.0));
        let shape = EdgeShape::from_paths(&[PathInput::new(p, None, Some(StrokeStyle::plain(2.0, Rgba8::BLACK)))]);
        assert!(paint_bucket(&shape, Point::new(50.0, 50.0), &FillStyle::Solid(blue()), 0.0, false).is_none());
        let filled = paint_bucket(&shape, Point::new(50.0, 50.0), &FillStyle::Solid(blue()), 8.0, false).expect("gap closed");
        assert_eq!(fill_color_at(&filled, Point::new(50.0, 50.0)), Some(blue()));
    }

    #[test]
    fn ink_bottle_restyles_a_stroke_and_outlines_a_fill() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), Some(2.0));
        let thick = StrokeStyle::plain(8.0, blue());
        let inked = ink_bottle(&shape, Point::new(0.0, 50.0), 2.0, &thick).expect("stroke");
        assert!(inked.strokes.contains(&thick));
        let plain = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), None);
        let outlined = ink_bottle(&plain, Point::new(50.0, 50.0), 2.0, &thick).expect("fill");
        assert!(matches!(outlined.hit_test(Point::new(0.0, 50.0), 1.0), Some(Hit::Stroke { .. })));
        assert!(ink_bottle(&plain, Point::new(500.0, 500.0), 2.0, &thick).is_none());
    }

    #[test]
    fn eraser_cuts_fills_and_strokes() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), Some(2.0));
        let path = [Point::new(-20.0, 50.0), Point::new(120.0, 50.0)];
        let spec = EraseSpec { points: &path, radius: 10.0, tip: TipShape::Round, fills: true, lines: true, only_fill_at: None };
        let Erased::Changed(cut) = erase(&shape, &spec) else { panic!("expected a change") };
        assert!(cut.fill_at(Point::new(50.0, 50.0)).is_none());
        assert_eq!(fill_color_at(&cut, Point::new(50.0, 20.0)), Some(red()));
        assert_eq!(fill_color_at(&cut, Point::new(50.0, 80.0)), Some(red()));
        assert!(cut.hit_test(Point::new(0.0, 50.0), 1.0).is_none(), "the left edge is erased at y = 50");
        assert!(cut.hit_test(Point::new(0.0, 10.0), 1.0).is_some());
    }

    #[test]
    fn eraser_modes_and_misses() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), Some(2.0));
        let path = [Point::new(50.0, 50.0)];
        let lines_only = EraseSpec { points: &path, radius: 10.0, tip: TipShape::Round, fills: false, lines: true, only_fill_at: None };
        assert_eq!(erase(&shape, &lines_only), Erased::Unchanged);
        let far = [Point::new(500.0, 500.0)];
        let spec = EraseSpec { points: &far, radius: 10.0, tip: TipShape::Round, fills: true, lines: true, only_fill_at: None };
        assert_eq!(erase(&shape, &spec), Erased::Unchanged);
        let all = [Point::new(50.0, 50.0)];
        let big = EraseSpec { points: &all, radius: 200.0, tip: TipShape::Round, fills: true, lines: true, only_fill_at: None };
        assert_eq!(erase(&shape, &big), Erased::Gone);
    }

    #[test]
    fn square_eraser_reaches_the_corners_a_round_one_misses() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), None);
        let at = [Point::new(50.0, 50.0)];
        let spot = Point::new(58.0, 58.0);
        let round = EraseSpec { points: &at, radius: 10.0, tip: TipShape::Round, fills: true, lines: true, only_fill_at: None };
        let square = EraseSpec { tip: TipShape::Square, ..round };
        let Erased::Changed(a) = erase(&shape, &round) else { panic!("round") };
        let Erased::Changed(b) = erase(&shape, &square) else { panic!("square") };
        assert!(a.fill_at(spot).is_some());
        assert!(b.fill_at(spot).is_none());
    }

    #[test]
    fn faucet_removes_the_whole_area_or_one_stroke_segment() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), Some(2.0));
        let Erased::Changed(no_fill) = faucet(&shape, Point::new(50.0, 50.0), 2.0, true, true) else { panic!("fill") };
        assert!(no_fill.fill_at(Point::new(50.0, 50.0)).is_none());
        assert!(no_fill.hit_test(Point::new(0.0, 50.0), 1.0).is_some(), "the outline stays");
        let Erased::Changed(no_edge) = faucet(&shape, Point::new(0.0, 50.0), 2.0, true, true) else { panic!("stroke") };
        assert!(no_edge.hit_test(Point::new(0.0, 50.0), 1.0).is_none() || no_edge.fill_at(Point::new(50.0, 50.0)).is_some());
        assert_eq!(faucet(&shape, Point::new(500.0, 500.0), 2.0, true, true), Erased::Unchanged);
        assert_eq!(faucet(&shape, Point::new(50.0, 50.0), 2.0, false, true), Erased::Unchanged);
    }

    #[test]
    fn brush_modes_clip_what_they_paint() {
        let blue = FillStyle::Solid(Rgba8::opaque(0, 0, 255));
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, Some(red()), None);
        let mut brush = BezPath::new();
        brush.move_to(Point::new(80.0, 40.0));
        brush.line_to(Point::new(140.0, 40.0));
        brush.line_to(Point::new(140.0, 60.0));
        brush.line_to(Point::new(80.0, 60.0));
        brush.close_path();
        let behind = paint_clipped(&shape, &brush, &blue, BrushMode::PaintBehind, Point::new(100.0, 50.0)).expect("behind");
        assert_eq!(fill_color_at(&behind, Point::new(90.0, 50.0)), Some(red()), "existing paint stays on top");
        assert_eq!(fill_color_at(&behind, Point::new(120.0, 50.0)), Some(Rgba8::opaque(0, 0, 255)));
        let inside = paint_clipped(&shape, &brush, &blue, BrushMode::PaintInside, Point::new(90.0, 50.0)).expect("inside");
        assert_eq!(fill_color_at(&inside, Point::new(90.0, 50.0)), Some(Rgba8::opaque(0, 0, 255)));
        assert!(inside.fill_at(Point::new(120.0, 50.0)).is_none(), "nothing is painted outside the fill");
        let selection = paint_clipped(&shape, &brush, &blue, BrushMode::PaintSelection, Point::new(90.0, 50.0)).expect("selection");
        assert_eq!(fill_color_at(&selection, Point::new(90.0, 50.0)), Some(Rgba8::opaque(0, 0, 255)));
        assert!(selection.fill_at(Point::new(120.0, 50.0)).is_none());
    }

    #[test]
    fn trim_keeps_untouched_curves_verbatim() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((10.0, 20.0), (30.0, 20.0), (40.0, 0.0));
        let out = trim_path(&p, Some(Rect::new(500.0, 500.0, 510.0, 510.0)), &|_| false);
        assert_eq!(out, p);
    }
}
