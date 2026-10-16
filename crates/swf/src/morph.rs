//! Shape tweens: `DefineMorphShape2`.
//!
//! A morph shape is two shapes with the same structure: the same styles in the same order and, record by record,
//! the same kind of edge. A placement with a `ratio` (0 ..= 65535) then shows the blend. [`MorphShapeDef::new`]
//! pairs up two [`ShapeDef`]s: edges that are straight in one shape and curved in the other are both written as
//! curves, and everything else that cannot be paired is an error naming the mismatch. (Working out which contours
//! of two arbitrary drawings belong together is the job of the tween engine, not of the file writer.)

use anim_display::Spread;

use crate::bits::PutBytes;
use crate::error::SwfError;
use crate::shape::{MAX_STYLES, ShapeDef, ShapeRecord, StyleChange, append_records, write_style_array};
use crate::style::{FillStyle, Gradient, LinePaint, LineStyle};
use crate::types::{Rect, put_rgba};

/// Two shapes prepared as the ends of a shape tween.
#[derive(Debug, Clone, PartialEq)]
pub struct MorphShapeDef {
    start: ShapeDef,
    end: ShapeDef,
    /// The end shape's records, rewritten to pair with the start records (moves and edges only).
    end_records: Vec<ShapeRecord>,
}

impl MorphShapeDef {
    /// Pairs `start` and `end`.
    ///
    /// # Errors
    /// [`SwfError::MorphMismatch`] when the shapes have different numbers of styles or of edges, when a pair of
    /// styles is of different kinds (or a gradient has reflect or repeat spread, which morph gradients cannot
    /// express), or when moves and edges do not line up.
    pub fn new(start: &ShapeDef, end: &ShapeDef) -> Result<Self, SwfError> {
        if start.fills.len() != end.fills.len() || start.lines.len() != end.lines.len() {
            return Err(SwfError::MorphMismatch("different numbers of fill or line styles"));
        }
        let mut start_records = Vec::with_capacity(start.records.len());
        let mut end_records = Vec::with_capacity(end.records.len());
        // Only moves and edges pair up; style changes without a move exist on the start side alone.
        let mut end_geometry = end.records.iter().filter(|r| !matches!(r, ShapeRecord::Style(s) if s.move_to.is_none()));
        for record in &start.records {
            match *record {
                ShapeRecord::Style(change) if change.move_to.is_none() => start_records.push(*record),
                ShapeRecord::Style(change) => {
                    let Some(&ShapeRecord::Style(other)) = end_geometry.next() else {
                        return Err(SwfError::MorphMismatch("a move in the start shape is not a move in the end shape"));
                    };
                    start_records.push(ShapeRecord::Style(change));
                    end_records.push(ShapeRecord::Style(StyleChange { move_to: other.move_to, ..StyleChange::default() }));
                }
                ShapeRecord::Line { .. } | ShapeRecord::Curve { .. } => {
                    let Some(other @ (ShapeRecord::Line { .. } | ShapeRecord::Curve { .. })) = end_geometry.next().copied() else {
                        return Err(SwfError::MorphMismatch("an edge in the start shape is not an edge in the end shape"));
                    };
                    match (*record, other) {
                        (ShapeRecord::Line { .. }, ShapeRecord::Line { .. }) | (ShapeRecord::Curve { .. }, ShapeRecord::Curve { .. }) => {
                            start_records.push(*record);
                            end_records.push(other);
                        }
                        (a, b) => {
                            start_records.push(as_curve(a));
                            end_records.push(as_curve(b));
                        }
                    }
                }
            }
        }
        if end_geometry.next().is_some() {
            return Err(SwfError::MorphMismatch("the end shape has more edges than the start shape"));
        }
        let mut paired = start.clone();
        paired.records = start_records;
        Ok(Self { start: paired, end: end.clone(), end_records })
    }

    /// The `DefineMorphShape2` body without the leading character id.
    ///
    /// # Errors
    /// [`SwfError::MorphMismatch`] for styles that cannot be morphed into each other and the errors of
    /// [`ShapeDef::encode`].
    pub fn encode(&self) -> Result<Vec<u8>, SwfError> {
        for count in [self.start.fills.len(), self.start.lines.len()] {
            if count > MAX_STYLES {
                return Err(SwfError::TooManyStyles(count));
            }
        }
        let mut out = Vec::new();
        for rect in [self.start.bounds, self.end.bounds, self.start.edge_bounds, self.end.edge_bounds] {
            rect.write(&mut out);
        }
        let non_scaling = self.start.lines.iter().any(|l| l.no_h_scale || l.no_v_scale);
        let scaling = self.start.lines.iter().any(|l| !(l.no_h_scale && l.no_v_scale));
        out.put_u8(u8::from(non_scaling).wrapping_shl(1) | u8::from(scaling));

        let mut after_offset = Vec::new();
        let mut failure = None;
        write_style_array(&mut after_offset, self.start.fills.len(), |buf| {
            for (a, b) in self.start.fills.iter().zip(&self.end.fills) {
                if let Err(e) = write_morph_fill(buf, a, b) {
                    failure.get_or_insert(e);
                }
            }
        });
        write_style_array(&mut after_offset, self.start.lines.len(), |buf| {
            for (a, b) in self.start.lines.iter().zip(&self.end.lines) {
                if let Err(e) = write_morph_line(buf, a, b) {
                    failure.get_or_insert(e);
                }
            }
        });
        if let Some(e) = failure {
            return Err(e);
        }
        append_records(&mut after_offset, &self.start.records, self.start.fills.len(), self.start.lines.len())?;
        let end_edges_at = u32::try_from(after_offset.len()).map_err(|_| SwfError::TooLarge(after_offset.len()))?;
        append_records(&mut after_offset, &self.end_records, self.start.fills.len(), self.start.lines.len())?;
        out.put_u32(end_edges_at);
        out.extend_from_slice(&after_offset);
        Ok(out)
    }

    /// Bounds of the shape at ratio 0 and at ratio 65535.
    #[must_use]
    pub const fn bounds(&self) -> (Rect, Rect) {
        (self.start.bounds, self.end.bounds)
    }
}

/// An edge as a curve: a straight edge becomes a curve whose control point is its midpoint.
const fn as_curve(record: ShapeRecord) -> ShapeRecord {
    match record {
        ShapeRecord::Line { dx, dy } => {
            let (half_x, half_y) = (dx / 2, dy / 2);
            ShapeRecord::Curve { control_dx: half_x, control_dy: half_y, anchor_dx: dx.saturating_sub(half_x), anchor_dy: dy.saturating_sub(half_y) }
        }
        other => other,
    }
}

fn write_morph_gradient(out: &mut Vec<u8>, a: &Gradient, b: &Gradient) -> Result<(), SwfError> {
    if a.records.len() != b.records.len() {
        return Err(SwfError::MorphMismatch("gradients with different numbers of stops"));
    }
    if a.spread != Spread::Pad || b.spread != Spread::Pad {
        return Err(SwfError::MorphMismatch("morph gradients cannot reflect or repeat"));
    }
    out.put_u8(u8::try_from(a.records.len()).unwrap_or(u8::MAX));
    for (ra, rb) in a.records.iter().zip(&b.records) {
        out.put_u8(ra.ratio);
        put_rgba(out, ra.color);
        out.put_u8(rb.ratio);
        put_rgba(out, rb.color);
    }
    Ok(())
}

fn write_morph_fill(out: &mut Vec<u8>, a: &FillStyle, b: &FillStyle) -> Result<(), SwfError> {
    match (a, b) {
        (FillStyle::Solid(x), FillStyle::Solid(y)) => {
            out.put_u8(0x00);
            put_rgba(out, *x);
            put_rgba(out, *y);
        }
        (FillStyle::Linear { matrix: ma, gradient: ga }, FillStyle::Linear { matrix: mb, gradient: gb }) => {
            out.put_u8(0x10);
            ma.write(out);
            mb.write(out);
            write_morph_gradient(out, ga, gb)?;
        }
        (FillStyle::Radial { matrix: ma, gradient: ga }, FillStyle::Radial { matrix: mb, gradient: gb }) => {
            out.put_u8(0x12);
            ma.write(out);
            mb.write(out);
            write_morph_gradient(out, ga, gb)?;
        }
        (FillStyle::Bitmap { id: ia, matrix: ma, repeat, smoothed }, FillStyle::Bitmap { id: ib, matrix: mb, .. }) if ia == ib => {
            out.put_u8(match (repeat, smoothed) {
                (true, true) => 0x40,
                (false, true) => 0x41,
                (true, false) => 0x42,
                (false, false) => 0x43,
            });
            out.put_u16(*ia);
            ma.write(out);
            mb.write(out);
        }
        (FillStyle::Focal { .. }, _) | (_, FillStyle::Focal { .. }) => return Err(SwfError::MorphMismatch("focal gradients cannot be morphed")),
        _ => return Err(SwfError::MorphMismatch("a fill style changes kind")),
    }
    Ok(())
}

fn write_morph_line(out: &mut Vec<u8>, a: &LineStyle, b: &LineStyle) -> Result<(), SwfError> {
    out.put_u16(a.width);
    out.put_u16(b.width);
    a.write_flags(out);
    match (&a.paint, &b.paint) {
        (LinePaint::Solid(x), LinePaint::Solid(y)) => {
            put_rgba(out, *x);
            put_rgba(out, *y);
        }
        (LinePaint::Fill(x), LinePaint::Fill(y)) => write_morph_fill(out, x, y)?,
        _ => return Err(SwfError::MorphMismatch("a line style changes between colour and fill")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use anim_core::Rgba8;
    use anim_core::kurbo::{BezPath, Rect as KRect, Shape};
    use anim_display::FillRule;

    use super::*;
    use crate::shape::filled_path_shape;

    fn square(x: f64, y: f64, size: f64, color: Rgba8) -> ShapeDef {
        filled_path_shape(&KRect::new(x, y, x + size, y + size).to_path(0.1), FillStyle::Solid(color), FillRule::NonZero).unwrap()
    }

    #[test]
    fn pairs_edges_and_writes_both_halves() {
        let a = square(0.0, 0.0, 10.0, Rgba8::opaque(255, 0, 0));
        let b = square(5.0, 5.0, 30.0, Rgba8::opaque(0, 0, 255));
        let morph = MorphShapeDef::new(&a, &b).unwrap();
        let body = morph.encode().unwrap();
        assert!(body.len() > 40);
        assert_eq!(morph.bounds().1.x_max, 700);
    }

    #[test]
    fn straight_edges_paired_with_curves_become_curves() {
        let a = square(0.0, 0.0, 10.0, Rgba8::BLACK);
        let mut circle = BezPath::new();
        circle.move_to((0.0, 0.0));
        circle.quad_to((5.0, -5.0), (10.0, 0.0));
        circle.quad_to((15.0, 5.0), (10.0, 10.0));
        circle.quad_to((5.0, 15.0), (0.0, 10.0));
        circle.quad_to((-5.0, 5.0), (0.0, 0.0));
        let b = filled_path_shape(&circle, FillStyle::Solid(Rgba8::BLACK), FillRule::NonZero).unwrap();
        let morph = MorphShapeDef::new(&a, &b).unwrap();
        assert!(morph.start.records.iter().all(|r| !matches!(r, ShapeRecord::Line { .. })));
        assert_eq!(morph.start.records.len(), morph.end_records.len());
        morph.encode().unwrap();
    }

    #[test]
    fn mismatches_are_reported() {
        let a = square(0.0, 0.0, 10.0, Rgba8::BLACK);
        let mut longer = KRect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1);
        longer.line_to((3.0, 3.0));
        let b = filled_path_shape(&longer, FillStyle::Solid(Rgba8::BLACK), FillRule::NonZero).unwrap();
        assert!(matches!(MorphShapeDef::new(&a, &b), Err(SwfError::MorphMismatch(_))));
        let no_fill = ShapeDef { fills: vec![], ..a.clone() };
        assert!(MorphShapeDef::new(&a, &no_fill).is_err());
        let radial = filled_path_shape(
            &KRect::new(0.0, 0.0, 10.0, 10.0).to_path(0.1),
            FillStyle::Linear {
                matrix: crate::types::Matrix::IDENTITY,
                gradient: Gradient::from_stops(&[], Spread::Reflect, anim_display::Interpolation::Rgb).0,
            },
            FillRule::NonZero,
        )
        .unwrap();
        // a solid fill cannot become a gradient, and reflecting gradients cannot be morphed at all
        assert!(MorphShapeDef::new(&a, &radial).unwrap().encode().is_err());
        assert!(MorphShapeDef::new(&radial, &radial).unwrap().encode().is_err());
    }
}
