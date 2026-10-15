//! Shapes: `SHAPEWITHSTYLE` records and the `DefineShape4` body.
//!
//! [`ShapeBuilder`] turns `kurbo` paths into SWF edge records: coordinates are rounded to twips once and every
//! edge is the difference of two rounded points (so rounding never accumulates), cubic curves become quadratics,
//! and edges longer than the 17-bit delta limit are split.

use anim_core::kurbo::{BezPath, CubicBez, ParamCurveExtrema, PathEl, Point, QuadBez, Shape};
use anim_core::num;
use anim_display::{Cap, FillRule, Join};

use crate::bits::{BitWriter, PutBytes, sb_bits, sb_bits_all, ub_bits};
use crate::error::SwfError;
use crate::style::{FillStyle, LineStyle};
use crate::types::{MAX_TWIPS, Rect, TWIPS_PER_PX, to_twips};

/// Largest style index a shape record can address (15 bits).
pub const MAX_STYLES: usize = 32_767;

/// Straight and curved edge deltas are signed 17-bit numbers.
const MIN_DELTA: i32 = -65_536;
const MAX_DELTA: i32 = 65_535;
/// Long straight edges are cut into pieces of at most this many twips per axis.
const STEP_LENGTH: i64 = 65_535;
const STEP_ROUNDING: i64 = 65_534;

/// Accuracy of the cubic to quadratic conversion, in pixels (a fifth of a twip).
const CUBIC_ACCURACY_PX: f64 = 0.01;

/// A style change in a shape's record list. `None` leaves the field as it was, `Some(0)` clears it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StyleChange {
    /// Absolute position in twips.
    pub move_to: Option<(i32, i32)>,
    /// Fill on the left of following edges (1-based index into the fill styles).
    pub fill0: Option<u32>,
    /// Fill on the right of following edges.
    pub fill1: Option<u32>,
    pub line: Option<u32>,
}

/// One record of a shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeRecord {
    Style(StyleChange),
    /// A straight edge by `(dx, dy)` twips.
    Line {
        dx: i32,
        dy: i32,
    },
    /// A quadratic edge: control point offset from the current point, then anchor offset from the control point.
    Curve {
        control_dx: i32,
        control_dy: i32,
        anchor_dx: i32,
        anchor_dy: i32,
    },
}

/// A complete shape, ready to be written as `DefineShape4`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeDef {
    pub fills: Vec<FillStyle>,
    pub lines: Vec<LineStyle>,
    pub records: Vec<ShapeRecord>,
    /// Bounds including strokes.
    pub bounds: Rect,
    /// Bounds of the fills alone.
    pub edge_bounds: Rect,
    /// Fills use the non-zero winding rule (otherwise even-odd). Needs SWF 10.
    pub non_zero_winding: bool,
}

impl ShapeDef {
    /// The `DefineShape4` body without the leading character id (so identical shapes compare equal).
    ///
    /// # Errors
    /// [`SwfError::TooManyStyles`] and [`SwfError::EdgeTooLong`] for hand-made shapes that break the format limits.
    pub fn encode(&self) -> Result<Vec<u8>, SwfError> {
        for count in [self.fills.len(), self.lines.len()] {
            if count > MAX_STYLES {
                return Err(SwfError::TooManyStyles(count));
            }
        }
        let mut out = Vec::new();
        self.bounds.write(&mut out);
        self.edge_bounds.write(&mut out);
        let non_scaling = self.lines.iter().any(|l| l.no_h_scale || l.no_v_scale);
        let scaling = self.lines.iter().any(|l| !(l.no_h_scale && l.no_v_scale));
        out.put_u8(u8::from(self.non_zero_winding).wrapping_shl(2) | u8::from(non_scaling).wrapping_shl(1) | u8::from(scaling));
        write_style_array(&mut out, self.fills.len(), |out| {
            for fill in &self.fills {
                fill.write(out);
            }
        });
        write_style_array(&mut out, self.lines.len(), |out| {
            for line in &self.lines {
                line.write(out);
            }
        });
        let fill_bits = ub_bits(u32::try_from(self.fills.len()).unwrap_or(0));
        let line_bits = ub_bits(u32::try_from(self.lines.len()).unwrap_or(0));
        let mut w = BitWriter::new();
        w.ub(fill_bits, 4);
        w.ub(line_bits, 4);
        for record in &self.records {
            write_record(&mut w, record, fill_bits, line_bits)?;
        }
        // EndShapeRecord: type flag 0 and five zero flags
        w.ub(0, 6);
        out.extend_from_slice(&w.into_bytes());
        Ok(out)
    }
}

/// `FILLSTYLEARRAY` / `LINESTYLEARRAY` count prefix, then the styles.
fn write_style_array(out: &mut Vec<u8>, count: usize, body: impl FnOnce(&mut Vec<u8>)) {
    match u8::try_from(count) {
        Ok(n) if n < 0xff => out.put_u8(n),
        _ => {
            out.put_u8(0xff);
            out.put_u16(u16::try_from(count).unwrap_or(u16::MAX));
        }
    }
    body(out);
}

fn write_record(w: &mut BitWriter, record: &ShapeRecord, fill_bits: u32, line_bits: u32) -> Result<(), SwfError> {
    match *record {
        ShapeRecord::Style(change) => {
            let StyleChange { move_to, fill0, fill1, line } = change;
            if move_to.is_none() && fill0.is_none() && fill1.is_none() && line.is_none() {
                return Ok(());
            }
            w.bit(false);
            w.bit(false); // no new style arrays
            w.bit(line.is_some());
            w.bit(fill1.is_some());
            w.bit(fill0.is_some());
            w.bit(move_to.is_some());
            if let Some((x, y)) = move_to {
                let n = sb_bits(x).max(sb_bits(y)).min(31);
                w.ub(n, 5);
                w.sb(x, n);
                w.sb(y, n);
            }
            for (style, bits) in [(fill0, fill_bits), (fill1, fill_bits), (line, line_bits)] {
                if let Some(index) = style {
                    w.ub(index, bits);
                }
            }
        }
        ShapeRecord::Line { dx, dy } => {
            check_delta(dx)?;
            check_delta(dy)?;
            let n = sb_bits(dx).max(sb_bits(dy)).max(2);
            w.bit(true);
            w.bit(true);
            w.ub(n.saturating_sub(2), 4);
            if dx != 0 && dy != 0 {
                w.bit(true);
                w.sb(dx, n);
                w.sb(dy, n);
            } else {
                w.bit(false);
                if dx == 0 {
                    w.bit(true);
                    w.sb(dy, n);
                } else {
                    w.bit(false);
                    w.sb(dx, n);
                }
            }
        }
        ShapeRecord::Curve { control_dx, control_dy, anchor_dx, anchor_dy } => {
            let values = [control_dx, control_dy, anchor_dx, anchor_dy];
            for v in values {
                check_delta(v)?;
            }
            let n = sb_bits_all(&values).max(2);
            w.bit(true);
            w.bit(false);
            w.ub(n.saturating_sub(2), 4);
            for v in values {
                w.sb(v, n);
            }
        }
    }
    Ok(())
}

const fn check_delta(v: i32) -> Result<(), SwfError> {
    if v < MIN_DELTA || v > MAX_DELTA { Err(SwfError::EdgeTooLong(v)) } else { Ok(()) }
}

/// Which side(s) of a path's edges carry which style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PathStyle {
    pub fill0: u32,
    pub fill1: u32,
    pub line: u32,
}

/// Builds a [`ShapeDef`] from styles and `kurbo` paths.
#[derive(Debug, Default)]
pub struct ShapeBuilder {
    fills: Vec<FillStyle>,
    lines: Vec<LineStyle>,
    records: Vec<ShapeRecord>,
    /// Current point in twips (the end of the last edge).
    pos: (i32, i32),
    /// Style state the records so far leave behind.
    state: PathStyle,
    edge_bounds: Option<Rect>,
    non_zero: bool,
}

impl ShapeBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Chooses the fill rule for all fills in this shape (the SWF flag is per shape).
    pub const fn set_fill_rule(&mut self, rule: FillRule) {
        self.non_zero = matches!(rule, FillRule::NonZero);
    }

    /// Adds a fill style and returns its 1-based index.
    ///
    /// # Errors
    /// [`SwfError::TooManyStyles`] past [`MAX_STYLES`].
    pub fn add_fill(&mut self, fill: FillStyle) -> Result<u32, SwfError> {
        if self.fills.len() >= MAX_STYLES {
            return Err(SwfError::TooManyStyles(self.fills.len().saturating_add(1)));
        }
        self.fills.push(fill);
        Ok(u32::try_from(self.fills.len()).unwrap_or(u32::MAX))
    }

    /// Adds a line style and returns its 1-based index.
    ///
    /// # Errors
    /// [`SwfError::TooManyStyles`] past [`MAX_STYLES`].
    pub fn add_line(&mut self, line: LineStyle) -> Result<u32, SwfError> {
        if self.lines.len() >= MAX_STYLES {
            return Err(SwfError::TooManyStyles(self.lines.len().saturating_add(1)));
        }
        self.lines.push(line);
        Ok(u32::try_from(self.lines.len()).unwrap_or(u32::MAX))
    }

    /// Appends `path` (in pixels, converted to twips here) with `style`. Subpaths that carry a fill are closed
    /// explicitly, because SWF fills need closed outlines.
    pub fn add_path(&mut self, path: &BezPath, style: PathStyle) {
        let filled = style.fill0 != 0 || style.fill1 != 0;
        // The start of the current subpath, in twips and in (sanitised) pixels, and the pixel position of the pen.
        let mut start: Option<(i32, i32)> = None;
        let mut start_px = Point::ORIGIN;
        let mut pen = Point::ORIGIN;
        let mut edges = 0_usize;
        for element in path.elements() {
            // A path that does not begin with a MoveTo starts where its first drawing command is.
            if start.is_none() && !matches!(element, PathEl::MoveTo(_) | PathEl::ClosePath) {
                let first = match *element {
                    PathEl::LineTo(first) | PathEl::QuadTo(first, _) | PathEl::CurveTo(first, _, _) => clean(first),
                    PathEl::MoveTo(_) | PathEl::ClosePath => Point::ORIGIN,
                };
                let q = twip_point(first);
                self.move_to(q, style);
                start = Some(q);
                start_px = first;
                pen = first;
            }
            match *element {
                PathEl::MoveTo(p) => {
                    self.close_if_filled(start, filled);
                    let p = clean(p);
                    let q = twip_point(p);
                    self.move_to(q, style);
                    start = Some(q);
                    start_px = p;
                    pen = p;
                    edges = 0;
                }
                PathEl::LineTo(p) => {
                    let p = clean(p);
                    edges = edges.saturating_add(self.line_to(twip_point(p), !filled && edges == 0));
                    pen = p;
                }
                PathEl::QuadTo(c, p) => {
                    let p = clean(p);
                    self.quad_to(twip_point(clean(c)), twip_point(p));
                    edges = edges.saturating_add(1);
                    pen = p;
                }
                PathEl::CurveTo(c1, c2, p) => {
                    let p = clean(p);
                    let cubic = CubicBez::new(pen, clean(c1), clean(c2), p);
                    for (_, _, quad) in cubic.to_quads(CUBIC_ACCURACY_PX) {
                        self.quad_to(twip_point(quad.p1), twip_point(quad.p2));
                        edges = edges.saturating_add(1);
                    }
                    // the last quadratic ends at p up to rounding; land exactly on the rounded end point
                    let end = twip_point(p);
                    if self.pos != end {
                        self.line_to(end, false);
                    }
                    pen = p;
                }
                PathEl::ClosePath => {
                    if let Some(s) = start
                        && self.pos != s
                    {
                        self.line_to(s, false);
                    }
                    pen = start_px;
                }
            }
        }
        self.close_if_filled(start, filled);
    }

    fn close_if_filled(&mut self, start: Option<(i32, i32)>, filled: bool) {
        if let Some(s) = start
            && filled
            && self.pos != s
        {
            self.line_to(s, false);
        }
    }

    fn move_to(&mut self, q: (i32, i32), style: PathStyle) {
        let change = StyleChange {
            move_to: Some(q),
            fill0: (self.state.fill0 != style.fill0).then_some(style.fill0),
            fill1: (self.state.fill1 != style.fill1).then_some(style.fill1),
            line: (self.state.line != style.line).then_some(style.line),
        };
        self.state = style;
        self.records.push(ShapeRecord::Style(change));
        self.pos = q;
        self.grow(q);
    }

    /// Adds straight edges to `q`, splitting those beyond the delta limit. Zero-length edges are dropped unless
    /// `keep_dot`; returns the number of edges added.
    fn line_to(&mut self, q: (i32, i32), keep_dot: bool) -> usize {
        let dx = q.0.saturating_sub(self.pos.0);
        let dy = q.1.saturating_sub(self.pos.1);
        if dx == 0 && dy == 0 {
            if keep_dot {
                self.records.push(ShapeRecord::Line { dx: 0, dy: 0 });
                return 1;
            }
            return 0;
        }
        let longest = i64::from(dx.unsigned_abs().max(dy.unsigned_abs()));
        let steps = longest.saturating_add(STEP_ROUNDING).checked_div(STEP_LENGTH).unwrap_or(1).max(1);
        let mut previous = self.pos;
        for i in 1..=steps {
            let target = if i == steps { q } else { (self.pos.0.saturating_add(split(dx, i, steps)), self.pos.1.saturating_add(split(dy, i, steps))) };
            self.records.push(ShapeRecord::Line { dx: target.0.saturating_sub(previous.0), dy: target.1.saturating_sub(previous.1) });
            previous = target;
        }
        self.pos = q;
        self.grow(q);
        usize::try_from(steps).unwrap_or(1)
    }

    fn quad_to(&mut self, control: (i32, i32), anchor: (i32, i32)) {
        self.quad_to_depth(control, anchor, 0);
    }

    fn quad_to_depth(&mut self, control: (i32, i32), anchor: (i32, i32), depth: u32) {
        let (cdx, cdy) = (control.0.saturating_sub(self.pos.0), control.1.saturating_sub(self.pos.1));
        let (adx, ady) = (anchor.0.saturating_sub(control.0), anchor.1.saturating_sub(control.1));
        let fits = [cdx, cdy, adx, ady].iter().all(|v| (MIN_DELTA..=MAX_DELTA).contains(v));
        if !fits && depth < 20 {
            // de Casteljau split at t = 1/2
            let mid = |a: (i32, i32), b: (i32, i32)| ((a.0.saturating_add(b.0)).div_euclid(2), (a.1.saturating_add(b.1)).div_euclid(2));
            let (q0, q1) = (mid(self.pos, control), mid(control, anchor));
            let m = mid(q0, q1);
            self.quad_to_depth(q0, m, depth.saturating_add(1));
            self.quad_to_depth(q1, anchor, depth.saturating_add(1));
            return;
        }
        let clamp = |v: i32| v.clamp(MIN_DELTA, MAX_DELTA);
        self.records.push(ShapeRecord::Curve { control_dx: clamp(cdx), control_dy: clamp(cdy), anchor_dx: clamp(adx), anchor_dy: clamp(ady) });
        let start = self.pos;
        self.pos = anchor;
        self.grow(anchor);
        self.grow_quad(start, control, anchor);
    }

    fn grow(&mut self, p: (i32, i32)) {
        let point = Rect::new(p.0, p.0, p.1, p.1);
        self.edge_bounds = Some(self.edge_bounds.map_or(point, |b| b.union(point)));
    }

    /// Extends the bounds by the extrema of a quadratic (its control point may lie outside the curve).
    fn grow_quad(&mut self, p0: (i32, i32), c: (i32, i32), p2: (i32, i32)) {
        let quad = QuadBez::new(
            Point::new(f64::from(p0.0), f64::from(p0.1)),
            Point::new(f64::from(c.0), f64::from(c.1)),
            Point::new(f64::from(p2.0), f64::from(p2.1)),
        );
        let b = ParamCurveExtrema::bounding_box(&quad);
        let lo = |v: f64| num::floor_f64_to_i32(v).clamp(-MAX_TWIPS, MAX_TWIPS);
        let hi = |v: f64| num::ceil_f64_to_i32(v).clamp(-MAX_TWIPS, MAX_TWIPS);
        let curve = Rect::new(lo(b.min_x()), hi(b.max_x()), lo(b.min_y()), hi(b.max_y()));
        self.edge_bounds = Some(self.edge_bounds.map_or(curve, |e| e.union(curve)));
    }

    /// Finishes the shape.
    #[must_use]
    pub fn finish(self) -> ShapeDef {
        let edge_bounds = self.edge_bounds.unwrap_or_default();
        let mut bounds = edge_bounds;
        let used_lines = self.records.iter().filter_map(|r| if let ShapeRecord::Style(s) = r { s.line } else { None }).filter(|l| *l != 0);
        let mut grow_by = 0_i32;
        for index in used_lines {
            if let Some(line) = usize::try_from(index).ok().and_then(|i| i.checked_sub(1)).and_then(|i| self.lines.get(i)) {
                let half = f64::from(line.width.max(1)) / 2.0;
                let factor = match (line.join, line.start_cap == Cap::Square || line.end_cap == Cap::Square) {
                    (Join::Miter, _) => line.miter_limit.clamp(1.0, 10.0),
                    (_, true) => std::f64::consts::SQRT_2,
                    _ => 1.0,
                };
                grow_by = grow_by.max(num::ceil_f64_to_i32(half * factor).saturating_add(1));
            }
        }
        if grow_by > 0 {
            bounds = bounds.inflate(grow_by);
        }
        ShapeDef { fills: self.fills, lines: self.lines, records: self.records, bounds, edge_bounds, non_zero_winding: self.non_zero }
    }
}

/// `value * i / n` with truncation toward zero, in 64 bits.
fn split(value: i32, i: i64, n: i64) -> i32 {
    i32::try_from(i64::from(value).saturating_mul(i).checked_div(n).unwrap_or(0)).unwrap_or(0)
}

fn twip_point(p: Point) -> (i32, i32) {
    (to_twips(p.x), to_twips(p.y))
}

/// Replaces `NaN` by 0 and clamps to the largest pixel coordinate SWF shapes can hold here, so that hostile
/// coordinates cannot blow up curve subdivision.
fn clean(p: Point) -> Point {
    let limit = f64::from(MAX_TWIPS) / TWIPS_PER_PX;
    let fix = |v: f64| if v.is_nan() { 0.0 } else { v.clamp(-limit, limit) };
    Point::new(fix(p.x), fix(p.y))
}

/// Whether a path's fill belongs on the right of its edges (`fill1`) rather than the left (`fill0`).
///
/// Clockwise paths in y-down coordinates (positive area) have their inside on the right.
#[must_use]
pub fn fill_side_is_right(path: &BezPath) -> bool {
    let area = Shape::area(path);
    area >= 0.0 || area.is_nan()
}

/// Convenience: a single filled path as a shape with one fill style.
///
/// # Errors
/// Propagates [`SwfError::TooManyStyles`] (cannot happen for a single style).
pub fn filled_path_shape(path: &BezPath, fill: FillStyle, rule: FillRule) -> Result<ShapeDef, SwfError> {
    let mut b = ShapeBuilder::new();
    b.set_fill_rule(rule);
    let id = b.add_fill(fill)?;
    let style = if fill_side_is_right(path) { PathStyle { fill1: id, ..PathStyle::default() } } else { PathStyle { fill0: id, ..PathStyle::default() } };
    b.add_path(path, style);
    Ok(b.finish())
}

#[cfg(test)]
mod tests {
    use anim_core::Rgba8;
    use anim_core::kurbo::Rect as KRect;

    use super::*;

    fn square(size: f64) -> BezPath {
        KRect::new(0.0, 0.0, size, size).to_path(0.1)
    }

    #[test]
    fn rectangle_becomes_move_and_four_edges() {
        let def = filled_path_shape(&square(10.0), FillStyle::Solid(Rgba8::opaque(255, 0, 0)), FillRule::NonZero).unwrap();
        assert_eq!(def.fills.len(), 1);
        assert_eq!(
            def.records,
            vec![
                ShapeRecord::Style(StyleChange { move_to: Some((0, 0)), fill0: None, fill1: Some(1), line: None }),
                ShapeRecord::Line { dx: 200, dy: 0 },
                ShapeRecord::Line { dx: 0, dy: 200 },
                ShapeRecord::Line { dx: -200, dy: 0 },
                ShapeRecord::Line { dx: 0, dy: -200 },
            ]
        );
        assert_eq!(def.edge_bounds, Rect::new(0, 200, 0, 200));
        assert!(def.non_zero_winding);
        assert!(def.encode().unwrap().len() > 20);
    }

    #[test]
    fn counter_clockwise_paths_use_the_left_fill() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((0.0, 10.0));
        path.line_to((10.0, 10.0));
        path.line_to((10.0, 0.0));
        path.close_path();
        assert!(!fill_side_is_right(&path));
        let def = filled_path_shape(&path, FillStyle::Solid(Rgba8::BLACK), FillRule::EvenOdd).unwrap();
        assert!(matches!(def.records[0], ShapeRecord::Style(StyleChange { fill0: Some(1), fill1: None, .. })));
        assert!(!def.non_zero_winding);
    }

    #[test]
    fn open_filled_paths_are_closed() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 0.0));
        path.line_to((10.0, 10.0));
        let def = filled_path_shape(&path, FillStyle::Solid(Rgba8::BLACK), FillRule::NonZero).unwrap();
        let last = def.records.last().unwrap();
        assert_eq!(*last, ShapeRecord::Line { dx: -200, dy: -200 });
    }

    #[test]
    fn long_edges_are_split_into_encodable_pieces() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10_000.0, 0.0)); // 200_000 twips
        path.line_to((10_000.0, 1.0));
        let def = filled_path_shape(&path, FillStyle::Solid(Rgba8::BLACK), FillRule::NonZero).unwrap();
        let total: i32 = def.records.iter().map(|r| if let ShapeRecord::Line { dx, .. } = r { *dx } else { 0 }).sum();
        // out 200_000, back along the closing diagonal
        assert_eq!(total, 0);
        for r in &def.records {
            if let ShapeRecord::Line { dx, dy } = r {
                assert!((MIN_DELTA..=MAX_DELTA).contains(dx) && (MIN_DELTA..=MAX_DELTA).contains(dy));
            }
        }
        def.encode().unwrap();
    }

    #[test]
    fn cubics_become_quadratics_ending_on_the_rounded_end_point() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.curve_to((0.0, 50.0), (100.0, 50.0), (100.0, 0.0));
        let mut b = ShapeBuilder::new();
        let fill = b.add_fill(FillStyle::Solid(Rgba8::BLACK)).unwrap();
        b.add_path(&path, PathStyle { fill1: fill, ..PathStyle::default() });
        let def = b.finish();
        let mut x = 0;
        let mut y = 0;
        let mut curves = 0;
        for r in &def.records {
            match *r {
                ShapeRecord::Curve { control_dx, control_dy, anchor_dx, anchor_dy } => {
                    x += control_dx + anchor_dx;
                    y += control_dy + anchor_dy;
                    curves += 1;
                }
                ShapeRecord::Line { dx, dy } => {
                    x += dx;
                    y += dy;
                }
                ShapeRecord::Style(_) => {}
            }
        }
        assert!(curves >= 2);
        // the path was closed back to the start
        assert_eq!((x, y), (0, 0));
        // the curve's apex is at y = 37.5 px = 750 twips
        assert!(def.edge_bounds.y_max >= 745 && def.edge_bounds.y_max <= 755, "{:?}", def.edge_bounds);
    }

    #[test]
    fn strokes_grow_the_bounds_and_dots_survive() {
        let mut path = BezPath::new();
        path.move_to((5.0, 5.0));
        path.line_to((5.0, 5.0));
        let mut b = ShapeBuilder::new();
        let line = b.add_line(LineStyle::solid(100, Rgba8::BLACK)).unwrap();
        b.add_path(&path, PathStyle { line, ..PathStyle::default() });
        let def = b.finish();
        assert!(def.records.contains(&ShapeRecord::Line { dx: 0, dy: 0 }));
        assert!(def.bounds.x_min < def.edge_bounds.x_min);
        assert_eq!(def.edge_bounds, Rect::new(100, 100, 100, 100));
    }

    #[test]
    fn hand_made_shapes_validate_limits() {
        let mut def = filled_path_shape(&square(1.0), FillStyle::Solid(Rgba8::BLACK), FillRule::NonZero).unwrap();
        def.records.push(ShapeRecord::Line { dx: 70_000, dy: 0 });
        assert!(matches!(def.encode(), Err(SwfError::EdgeTooLong(70_000))));
    }

    #[test]
    fn nan_and_infinite_coordinates_do_not_panic() {
        let mut path = BezPath::new();
        path.move_to((f64::NAN, f64::INFINITY));
        path.line_to((f64::NEG_INFINITY, 1e300));
        path.quad_to((1e300, 1e300), (-1e300, 5.0));
        path.curve_to((1e300, 1.0), (f64::NAN, 2.0), (3.0, 4.0));
        path.close_path();
        let def = filled_path_shape(&path, FillStyle::Solid(Rgba8::BLACK), FillRule::NonZero).unwrap();
        def.encode().unwrap();
    }
}
