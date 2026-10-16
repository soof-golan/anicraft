//! Modify > Shape and Modify > Combine Objects: geometry on edge shapes.
//!
//! Combine Objects works on the filled bodies of whole objects ([`Body`]); the Shape commands (smooth, straighten,
//! optimize curves, convert lines to fills, expand fill, soften fill edges) work on the fills and strokes inside one
//! shape. Everything here is a pure function of geometry; the commands that apply it to the document live in
//! `anicraft-cli`.

use anim_core::kurbo::{CubicBez, ParamCurve};
use anim_core::num::{round_f64_to_u32, usize_to_f64};
use anim_core::{Affine, BezPath, Point, Rgba8};
use anim_display::{Cap as DisplayCap, Join as DisplayJoin};
use anim_shape::{EdgeShape, FillStyle, PathInput, StrokeStyle};
use vectorcraft_geom::{FillRule, PathData, SubPath};
use vectorcraft_pathops::{
    BoolOp, Cap, Join, SimplifyOptions, boolean, offset_path, outline_stroke, remove_redundant_points, simplify_with, smooth, unite_all,
};

use crate::shapeops::{Parts, decompose, merge, rebuild};

/// The area an object covers, in stage coordinates, with the styles the object paints it with.
#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    /// The covered area (contours with the even-odd rule).
    pub path: BezPath,
    /// The object's main fill.
    pub fill: Option<FillStyle>,
    /// The object's main stroke, drawn along the outline of the area.
    pub stroke: Option<StrokeStyle>,
}

/// The four Combine Objects commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombineOp {
    /// One object from the visible parts of all of them.
    Union,
    /// One object from the area all of them cover.
    Intersect,
    /// The topmost object cuts its shape out of the objects below it and disappears.
    Punch,
    /// The topmost object cuts the objects below it down to its own shape and disappears.
    Crop,
}

const fn display_cap(cap: DisplayCap) -> Cap {
    match cap {
        DisplayCap::None => Cap::Butt,
        DisplayCap::Round => Cap::Round,
        DisplayCap::Square => Cap::Square,
    }
}

const fn display_join(join: DisplayJoin) -> Join {
    match join {
        DisplayJoin::Miter => Join::Miter,
        DisplayJoin::Round => Join::Round,
        DisplayJoin::Bevel => Join::Bevel,
    }
}

fn united(paths: &[PathData]) -> PathData {
    let refs: Vec<(&PathData, FillRule)> = paths.iter().map(|p| (p, FillRule::EvenOdd)).collect();
    unite_all(&refs)
}

/// The area of a stroke: its outline, or a hairline's thin one.
fn stroke_area(style: &StrokeStyle, path: &BezPath) -> PathData {
    let s = style.solid();
    outline_stroke(&PathData::from_bezpath(path), s.weight.max(0.25), display_cap(s.caps), display_join(s.joints), s.miter_limit)
}

/// The body of a shape placed with `matrix`: the union of its fills (or, for a line drawing, the area its strokes
/// paint), with the first fill and stroke as styles. `None` for an empty shape.
#[must_use]
pub fn body_of(shape: &EdgeShape, matrix: Affine) -> Option<Body> {
    let parts = decompose(shape);
    let fills: Vec<PathData> = parts.fills.iter().map(|(_, p)| PathData::from_bezpath(p)).collect();
    let first_stroke = parts.strokes.first().map(|(s, _)| s.clone());
    if fills.is_empty() {
        let areas: Vec<PathData> = parts.strokes.iter().map(|(s, p)| stroke_area(s, p)).collect();
        let area = united(&areas);
        let style = first_stroke?;
        let fill = style.solid().fill.clone();
        return (!area.is_empty()).then(|| Body { path: (matrix * area.to_bezpath()), fill: Some(fill), stroke: None });
    }
    let area = united(&fills);
    let fill = parts.fills.first().map(|(f, _)| f.clone());
    (!area.is_empty()).then(|| Body { path: matrix * area.to_bezpath(), fill, stroke: first_stroke })
}

/// The shape that draws a body.
#[must_use]
pub fn shape_of(body: &Body) -> EdgeShape {
    EdgeShape::from_paths(&[PathInput::new(body.path.clone(), body.fill.clone(), body.stroke.clone())])
}

/// Applies a Combine Objects command to the bodies of the selected objects, bottom-most first.
///
/// Returns the bodies that remain, each with the index of the object it came from; objects that end up empty are
/// dropped.
#[must_use]
pub fn combine(op: CombineOp, bodies: &[Body]) -> Vec<(usize, Body)> {
    let data: Vec<PathData> = bodies.iter().map(|b| PathData::from_bezpath(&b.path)).collect();
    let Some(top) = bodies.len().checked_sub(1) else { return Vec::new() };
    let keep = |i: usize, path: &PathData| -> Option<(usize, Body)> {
        let source = bodies.get(i)?;
        (!path.is_empty()).then(|| (i, Body { path: path.to_bezpath(), fill: source.fill.clone(), stroke: source.stroke.clone() }))
    };
    match op {
        CombineOp::Union => keep(top, &united(&data)).into_iter().collect(),
        CombineOp::Intersect => {
            let mut iter = data.iter();
            let Some(first) = iter.next() else { return Vec::new() };
            let mut acc = first.clone();
            for next in iter {
                acc = boolean(&acc, FillRule::EvenOdd, next, FillRule::EvenOdd, BoolOp::Intersect);
            }
            keep(top, &acc).into_iter().collect()
        }
        CombineOp::Punch | CombineOp::Crop => {
            let Some(cutter) = data.get(top) else { return Vec::new() };
            let bop = if op == CombineOp::Punch { BoolOp::Difference } else { BoolOp::Intersect };
            data.iter().take(top).enumerate().filter_map(|(i, d)| keep(i, &boolean(d, FillRule::EvenOdd, cutter, FillRule::EvenOdd, bop))).collect()
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Smooth, Straighten, Optimize
// ---------------------------------------------------------------------------------------------------------------------

/// Which curve command to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveEdit {
    /// Fewer anchors and softer handles.
    Smooth,
    /// Nearly straight segments become straight and freehand outlines turn into the shape they were meant to be.
    Straighten,
    /// Refit the curves with fewer anchors.
    Optimize,
}

/// The result of a curve command.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveReport {
    pub shape: EdgeShape,
    /// Anchors before and after.
    pub before: usize,
    pub after: usize,
}

fn deviation(c: &CubicBez) -> f64 {
    let chord = c.p3 - c.p0;
    let len = chord.hypot();
    let off = |p: Point| {
        if len < 1.0e-9 { (p - c.p0).hypot() } else { (p - c.p0).cross(chord).abs() / len }
    };
    off(c.p1).max(off(c.p2))
}

/// Segments whose control points lie within `tolerance` of the chord become lines.
fn straighten_segments(path: &PathData, tolerance: f64) -> PathData {
    let mut out = path.clone();
    for sp in &mut out.subpaths {
        let n = sp.anchors.len();
        if n < 2 {
            continue;
        }
        let count = sp.segment_count();
        for i in 0..count {
            if sp.segment_is_line(i) || deviation(&sp.segment(i)) > tolerance {
                continue;
            }
            let next = i.saturating_add(1).checked_rem(n).unwrap_or(0);
            if let Some(a) = sp.anchors.get_mut(i) {
                a.h_out = a.p;
            }
            if let Some(a) = sp.anchors.get_mut(next) {
                a.h_in = a.p;
            }
        }
    }
    out
}

/// A closed freehand outline replaced by the clean shape it resembles, when it resembles one.
fn recognised(sp: &SubPath) -> Option<SubPath> {
    use vectorcraft_geom::recognize::{Recognized, recognize};
    use vectorcraft_geom::shapes;
    if !sp.closed || sp.anchors.len() < 5 {
        return None;
    }
    let mut pts = Vec::new();
    for i in 0..sp.segment_count() {
        let c = sp.segment(i);
        for k in 0..8_u32 {
            pts.push(c.eval(f64::from(k) / 8.0));
        }
    }
    let path = match recognize(&pts)? {
        Recognized::Rectangle { rect, rotation } => shapes::rectangle(rect).transformed(Affine::rotate_about(rotation.to_radians(), rect.center())),
        Recognized::Ellipse { rect, rotation } => shapes::ellipse(rect).transformed(Affine::rotate_about(rotation.to_radians(), rect.center())),
        Recognized::Polygon { center, radius, sides, rotation } => shapes::polygon(center, radius, sides, rotation),
        Recognized::Line { .. } | Recognized::Scribble(_) => return None,
    };
    path.subpaths.into_iter().next()
}

fn edit_path(path: &BezPath, op: CurveEdit, amount: f64, passes: u32) -> PathData {
    let amount = if amount.is_finite() { amount.clamp(0.0, 1.0) } else { 0.5 };
    let mut data = PathData::from_bezpath(path);
    for _ in 0..passes.max(1) {
        data = match op {
            CurveEdit::Smooth => {
                let simplified =
                    simplify_with(&data, &SimplifyOptions { tolerance: 1.8_f64.mul_add(amount, 0.2), corner_angle_deg: 45.0, straight_lines: false });
                smooth(&simplified, 0.5_f64.mul_add(amount, 0.5))
            }
            CurveEdit::Optimize => {
                simplify_with(&data, &SimplifyOptions { tolerance: 3.0_f64.mul_add(amount, 0.1), corner_angle_deg: 30.0, straight_lines: false })
            }
            CurveEdit::Straighten => {
                let mut subpaths = Vec::new();
                for sp in &data.subpaths {
                    match recognised(sp) {
                        Some(clean) => subpaths.push(clean),
                        None => subpaths.extend(
                            remove_redundant_points(&straighten_segments(&PathData::new(vec![sp.clone()]), 4.0_f64.mul_add(amount, 0.5)), 0.25 + amount)
                                .subpaths,
                        ),
                    }
                }
                PathData::new(subpaths)
            }
        };
    }
    data
}

/// Smooths, straightens or optimizes every fill and stroke of a shape. `amount` runs from 0 (gentle) to 1 (strong).
#[must_use]
pub fn edit_curves(shape: &EdgeShape, op: CurveEdit, amount: f64, passes: u32) -> CurveReport {
    let parts = decompose(shape);
    let count = |p: &BezPath| PathData::from_bezpath(p).anchor_count();
    let before = parts.fills.iter().map(|(_, p)| count(p)).chain(parts.strokes.iter().map(|(_, p)| count(p))).sum();
    let edited = Parts {
        fills: parts.fills.iter().map(|(s, p)| (s.clone(), edit_path(p, op, amount, passes).to_bezpath())).collect(),
        strokes: parts.strokes.iter().map(|(s, p)| (s.clone(), edit_path(p, op, amount, passes).to_bezpath())).collect(),
    };
    let after = edited.fills.iter().map(|(_, p)| count(p)).chain(edited.strokes.iter().map(|(_, p)| count(p))).sum();
    CurveReport { shape: rebuild(&edited), before, after }
}

// ---------------------------------------------------------------------------------------------------------------------
// Lines to fills, expand, soften
// ---------------------------------------------------------------------------------------------------------------------

fn fills_only(parts: &Parts) -> EdgeShape {
    rebuild(&Parts { fills: parts.fills.clone(), strokes: Vec::new() })
}

fn single_fill(style: &FillStyle, path: &BezPath) -> EdgeShape {
    rebuild(&Parts { fills: vec![(style.clone(), path.clone())], strokes: Vec::new() })
}

/// Turns every stroke into a filled area painted with the stroke's colour. Fills keep their place underneath.
#[must_use]
pub fn lines_to_fills(shape: &EdgeShape) -> EdgeShape {
    let parts = decompose(shape);
    let mut out = fills_only(&parts);
    for (style, path) in &parts.strokes {
        let area = stroke_area(style, path);
        if !area.is_empty() {
            out = merge(&out, &single_fill(&style.solid().fill, &area.to_bezpath()));
        }
    }
    out
}

fn offset(path: &BezPath, distance: f64) -> BezPath {
    offset_path(&PathData::from_bezpath(path), distance, Join::Round, 4.0).to_bezpath()
}

/// Grows (`distance` > 0) or shrinks (< 0) every fill by `distance` pixels. Grown fills paint over their neighbours
/// in the order of the shape; strokes stay as they are, on top.
#[must_use]
pub fn expand_fills(shape: &EdgeShape, distance: f64) -> EdgeShape {
    let parts = decompose(shape);
    let mut out = EdgeShape::default();
    for (style, path) in &parts.fills {
        let grown = offset(path, distance);
        if !grown.elements().is_empty() {
            out = merge(&out, &single_fill(style, &grown));
        }
    }
    if parts.strokes.is_empty() { out } else { merge(&out, &rebuild(&Parts { fills: Vec::new(), strokes: parts.strokes })) }
}

/// Fades the edges of every solid fill over `distance` pixels in `steps` bands.
///
/// Outwards (`distance` > 0) the fill keeps its size and gets translucent bands around it, inwards (< 0) it shrinks
/// into translucent bands.
#[must_use]
pub fn soften_fills(shape: &EdgeShape, distance: f64, steps: u32) -> EdgeShape {
    let steps = steps.clamp(1, 64);
    let parts = decompose(shape);
    let mut out = EdgeShape::default();
    let n = f64::from(steps);
    for (style, path) in &parts.fills {
        let FillStyle::Solid(color) = style else {
            out = merge(&out, &single_fill(style, path));
            continue;
        };
        let alpha_at = |k: u32| -> Rgba8 {
            let level = f64::from(k.saturating_add(1)) / (n + 1.0);
            let a = (f64::from(color.a) * level).clamp(0.0, 255.0);
            Rgba8 { a: u8::try_from(round_f64_to_u32(a).min(255)).unwrap_or(255), ..*color }
        };
        if distance >= 0.0 {
            // Outermost band first, the fill itself last.
            for band in (1..=steps).rev() {
                let grown = offset(path, distance * f64::from(band) / n);
                let faded = FillStyle::Solid(alpha_at(steps.saturating_sub(band)));
                if !grown.elements().is_empty() {
                    out = merge(&out, &single_fill(&faded, &grown));
                }
            }
            out = merge(&out, &single_fill(style, path));
        } else {
            for level in 0..=steps {
                let inner = if level == 0 { path.clone() } else { offset(path, distance * f64::from(level) / n) };
                let faded = if level == steps { style.clone() } else { FillStyle::Solid(alpha_at(level)) };
                if !inner.elements().is_empty() {
                    out = merge(&out, &single_fill(&faded, &inner));
                }
            }
        }
    }
    if parts.strokes.is_empty() { out } else { merge(&out, &rebuild(&Parts { fills: Vec::new(), strokes: parts.strokes })) }
}

/// Total area of the fills of a shape (square pixels), for reports and tests.
#[must_use]
pub fn fill_area(shape: &EdgeShape) -> f64 {
    decompose(shape).fills.iter().map(|(_, p)| vectorcraft_pathops::area(&PathData::from_bezpath(p), FillRule::EvenOdd)).sum()
}

/// Anchors of a shape's outlines, for reports.
#[must_use]
pub fn anchor_total(shape: &EdgeShape) -> f64 {
    let parts = decompose(shape);
    usize_to_f64(parts.fills.iter().map(|(_, p)| p).chain(parts.strokes.iter().map(|(_, p)| p)).map(|p| PathData::from_bezpath(p).anchor_count()).sum())
}

#[cfg(test)]
mod tests {
    use anim_core::kurbo::{Rect, Shape as _};
    use anim_shape::StrokeStyle;

    use super::*;

    fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    fn solid(r: u8, g: u8, b: u8) -> FillStyle {
        FillStyle::Solid(Rgba8 { r, g, b, a: 255 })
    }

    fn rect_shape(x0: f64, y0: f64, x1: f64, y1: f64, fill: FillStyle, stroke: Option<StrokeStyle>) -> EdgeShape {
        EdgeShape::from_paths(&[PathInput::new(rect_path(x0, y0, x1, y1), Some(fill), stroke)])
    }

    fn area_of(path: &BezPath) -> f64 {
        vectorcraft_pathops::area(&PathData::from_bezpath(path), FillRule::EvenOdd)
    }

    fn body(x0: f64, y0: f64, x1: f64, y1: f64, fill: FillStyle) -> Body {
        body_of(&rect_shape(x0, y0, x1, y1, fill, None), Affine::IDENTITY).unwrap()
    }

    #[test]
    fn union_intersect_punch_and_crop_follow_the_overlap() {
        let bottom = body(0.0, 0.0, 100.0, 100.0, solid(255, 0, 0));
        let top = body(50.0, 50.0, 150.0, 150.0, solid(0, 0, 255));
        let both = [bottom.clone(), top.clone()];

        let union = combine(CombineOp::Union, &both);
        assert_eq!(union.len(), 1);
        assert!((area_of(&union[0].1.path) - 17_500.0).abs() < 1.0);
        assert_eq!(union[0].1.fill, top.fill, "the union takes the topmost object's fill");

        let intersect = combine(CombineOp::Intersect, &both);
        assert_eq!(intersect.len(), 1);
        assert!((area_of(&intersect[0].1.path) - 2_500.0).abs() < 1.0);

        let punch = combine(CombineOp::Punch, &both);
        assert_eq!(punch.len(), 1);
        assert_eq!(punch[0].0, 0);
        assert!((area_of(&punch[0].1.path) - 7_500.0).abs() < 1.0);
        assert_eq!(punch[0].1.fill, bottom.fill, "the punched object keeps its own fill");

        let crop = combine(CombineOp::Crop, &both);
        assert_eq!(crop.len(), 1);
        assert!((area_of(&crop[0].1.path) - 2_500.0).abs() < 1.0);
        assert_eq!(crop[0].1.fill, bottom.fill);
    }

    #[test]
    fn disjoint_objects_intersect_to_nothing() {
        let a = body(0.0, 0.0, 10.0, 10.0, solid(1, 2, 3));
        let b = body(50.0, 50.0, 60.0, 60.0, solid(4, 5, 6));
        assert_eq!(combine(CombineOp::Intersect, &[a, b]).len(), 0);
        assert_eq!(combine(CombineOp::Union, &[]).len(), 0);
    }

    #[test]
    fn optimizing_a_dense_outline_removes_anchors_and_keeps_the_area() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        for i in 1..=50 {
            path.line_to((f64::from(i) * 2.0, 0.0));
        }
        path.line_to((100.0, 80.0));
        path.line_to((0.0, 80.0));
        path.close_path();
        let shape = EdgeShape::from_paths(&[PathInput::new(path, Some(solid(9, 9, 9)), None)]);
        let report = edit_curves(&shape, CurveEdit::Optimize, 0.5, 1);
        assert!(report.after < report.before, "{} -> {}", report.before, report.after);
        assert!((fill_area(&report.shape) - 8_000.0).abs() < 30.0);
    }

    #[test]
    fn straightening_a_wobbly_rectangle_makes_a_rectangle() {
        let wobble = |t: f64| (t * 7.0).sin() * 0.4;
        let mut pts = Vec::new();
        for i in 0..20 {
            pts.push(Point::new(f64::from(i) * 5.0, wobble(f64::from(i))));
        }
        for i in 0..12 {
            pts.push(Point::new(100.0 + wobble(f64::from(i)), f64::from(i) * 5.0));
        }
        for i in 0..20 {
            pts.push(Point::new(100.0 - f64::from(i) * 5.0, 60.0 + wobble(f64::from(i))));
        }
        for i in 0..12 {
            pts.push(Point::new(wobble(f64::from(i)), 60.0 - f64::from(i) * 5.0));
        }
        let mut path = BezPath::new();
        path.move_to(pts[0]);
        for p in &pts[1..] {
            path.line_to(*p);
        }
        path.close_path();
        let shape = EdgeShape::from_paths(&[PathInput::new(path, Some(solid(9, 9, 9)), None)]);
        let report = edit_curves(&shape, CurveEdit::Straighten, 0.5, 1);
        assert!(report.after <= 8, "{} -> {}", report.before, report.after);
        assert!((fill_area(&report.shape) - 6_000.0).abs() < 200.0);
    }

    #[test]
    fn lines_become_fills_that_cover_the_stroke_width() {
        let mut path = BezPath::new();
        path.move_to((0.0, 10.0));
        path.line_to((100.0, 10.0));
        let line = EdgeShape::from_paths(&[PathInput::new(path, None, Some(StrokeStyle::plain(10.0, Rgba8 { r: 0, g: 0, b: 0, a: 255 })))]);
        assert_eq!(decompose(&line).fills.len(), 0);
        let filled = lines_to_fills(&line);
        let parts = decompose(&filled);
        assert_eq!(parts.strokes.len(), 0);
        let area = fill_area(&filled);
        assert!(area > 900.0 && area < 1_300.0, "{area}");
    }

    #[test]
    fn expanding_and_insetting_change_the_area() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, solid(5, 5, 5), None);
        let bigger = expand_fills(&shape, 10.0);
        assert!(fill_area(&bigger) > 12_000.0, "{}", fill_area(&bigger));
        let smaller = expand_fills(&shape, -10.0);
        assert!((fill_area(&smaller) - 6_400.0).abs() < 100.0, "{}", fill_area(&smaller));
    }

    #[test]
    fn softened_edges_are_translucent_bands_around_the_fill() {
        let shape = rect_shape(0.0, 0.0, 100.0, 100.0, solid(200, 0, 0), None);
        let soft = soften_fills(&shape, 12.0, 4);
        let parts = decompose(&soft);
        assert!(parts.fills.len() >= 5, "{}", parts.fills.len());
        let alphas: Vec<u8> = parts.fills.iter().filter_map(|(f, _)| f.solid_color()).map(|c| c.a).collect();
        assert!(alphas.contains(&255));
        assert!(alphas.iter().any(|a| *a < 255));
        assert!(fill_area(&soft) > 10_000.0 + 1_000.0);
        let inward = soften_fills(&shape, -12.0, 3);
        assert!(decompose(&inward).fills.len() >= 4);
    }
}
