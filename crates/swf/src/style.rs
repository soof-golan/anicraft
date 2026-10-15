//! Fill styles (solid, gradients, bitmaps) and `LINESTYLE2` line styles, as SWF stores them.

use anim_core::{Rgba8, num};
use anim_display::{Cap, GradientStop, Interpolation, Join, Spread};

use crate::bits::PutBytes;
use crate::types::{Matrix, put_rgba, to_fixed8};

/// SWF 8+ allows at most 15 gradient records.
pub const MAX_GRADIENT_RECORDS: usize = 15;

/// More stops than this are decimated before the greedy reduction, which is quadratic.
const HARD_CAP: usize = 256;

/// One gradient stop: `ratio` 0 ..= 255 along the gradient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GradientRecord {
    pub ratio: u8,
    pub color: Rgba8,
}

/// A SWF gradient: spread, interpolation space and 1 ..= 15 records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gradient {
    pub spread: Spread,
    pub interpolation: Interpolation,
    pub records: Vec<GradientRecord>,
}

impl Gradient {
    /// Builds a gradient from display-list stops: sorts them, clamps offsets, and reduces them to
    /// [`MAX_GRADIENT_RECORDS`] by repeatedly dropping the interior stop that the straight blend of its neighbours
    /// reproduces best. Returns the gradient and whether stops had to be dropped.
    #[must_use]
    pub fn from_stops(stops: &[GradientStop], spread: Spread, interpolation: Interpolation) -> (Self, bool) {
        let mut sorted: Vec<GradientStop> =
            stops.iter().map(|s| GradientStop { offset: if s.offset.is_nan() { 0.0 } else { s.offset.clamp(0.0, 1.0) }, color: s.color }).collect();
        sorted.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        let mut reduced = false;
        // Decimate absurd stop counts first so the quadratic reduction below stays cheap.
        if sorted.len() > HARD_CAP {
            reduced = true;
            let last = sorted.len().saturating_sub(1);
            let step = |i: usize| i.saturating_mul(last).checked_div(HARD_CAP.saturating_sub(1)).unwrap_or(0);
            sorted = (0..HARD_CAP).filter_map(|i| sorted.get(step(i)).copied()).collect();
        }
        while sorted.len() > MAX_GRADIENT_RECORDS {
            reduced = true;
            let victim = (1..sorted.len().saturating_sub(1)).min_by_key(|&i| removal_error(&sorted, i)).unwrap_or(1);
            sorted.remove(victim);
        }
        let mut records: Vec<GradientRecord> = sorted.iter().map(|s| GradientRecord { ratio: num::unit_f32_to_u8(s.offset), color: s.color }).collect();
        match records.as_slice() {
            [] => records = vec![GradientRecord { ratio: 0, color: Rgba8::TRANSPARENT }, GradientRecord { ratio: 255, color: Rgba8::TRANSPARENT }],
            [only] => {
                let color = only.color;
                records = vec![GradientRecord { ratio: 0, color }, GradientRecord { ratio: 255, color }];
            }
            _ => {}
        }
        (Self { spread, interpolation, records }, reduced)
    }

    fn write(&self, out: &mut Vec<u8>) {
        let spread = match self.spread {
            Spread::Pad => 0_u8,
            Spread::Reflect => 1,
            Spread::Repeat => 2,
        };
        let interpolation = match self.interpolation {
            Interpolation::Rgb => 0_u8,
            Interpolation::LinearRgb => 1,
        };
        let count = u8::try_from(self.records.len().min(MAX_GRADIENT_RECORDS)).unwrap_or(2);
        out.put_u8(spread.wrapping_shl(6) | interpolation.wrapping_shl(4) | count);
        for record in self.records.iter().take(MAX_GRADIENT_RECORDS) {
            out.put_u8(record.ratio);
            put_rgba(out, record.color);
        }
    }
}

/// How badly dropping stop `i` would change the gradient: the colour distance between it and the blend of its
/// neighbours at its offset, scaled to an integer so it can be a sort key.
fn removal_error(stops: &[GradientStop], i: usize) -> u32 {
    let (Some(before), Some(here), Some(after)) = (stops.get(i.saturating_sub(1)), stops.get(i), stops.get(i.saturating_add(1))) else { return u32::MAX };
    let span = after.offset - before.offset;
    let t = if span > f32::EPSILON { (here.offset - before.offset) / span } else { 0.5 };
    let channels = |a: u8, mid: u8, b: u8| {
        let blend = (f32::from(b) - f32::from(a)).mul_add(t, f32::from(a));
        (f32::from(mid) - blend).abs()
    };
    let error = channels(before.color.r, here.color.r, after.color.r)
        + channels(before.color.g, here.color.g, after.color.g)
        + channels(before.color.b, here.color.b, after.color.b)
        + channels(before.color.a, here.color.a, after.color.a);
    num::round_f64_to_u32(f64::from(error) * 16.0)
}

/// A fill style as the SWF stores it.
#[derive(Debug, Clone, PartialEq)]
pub enum FillStyle {
    Solid(Rgba8),
    /// Gradient along the x axis of gradient space (±16384 twips).
    Linear {
        matrix: Matrix,
        gradient: Gradient,
    },
    /// Gradient over the circle of radius 16384 twips.
    Radial {
        matrix: Matrix,
        gradient: Gradient,
    },
    /// A radial gradient whose focus sits on the x axis at `focal` (-1.0 ..= 1.0) of the radius.
    Focal {
        matrix: Matrix,
        gradient: Gradient,
        focal: f64,
    },
    /// A bitmap character; the matrix maps bitmap pixels to twips.
    Bitmap {
        id: u16,
        matrix: Matrix,
        repeat: bool,
        smoothed: bool,
    },
}

impl FillStyle {
    /// Appends the `FILLSTYLE` record (with `RGBA` colours, as `DefineShape3` and later use).
    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Solid(color) => {
                out.put_u8(0x00);
                put_rgba(out, *color);
            }
            Self::Linear { matrix, gradient } => {
                out.put_u8(0x10);
                matrix.write(out);
                gradient.write(out);
            }
            Self::Radial { matrix, gradient } => {
                out.put_u8(0x12);
                matrix.write(out);
                gradient.write(out);
            }
            Self::Focal { matrix, gradient, focal } => {
                out.put_u8(0x13);
                matrix.write(out);
                gradient.write(out);
                out.put_i16(to_fixed8(focal.clamp(-1.0, 1.0)));
            }
            Self::Bitmap { id, matrix, repeat, smoothed } => {
                out.put_u8(match (repeat, smoothed) {
                    (true, true) => 0x40,
                    (false, true) => 0x41,
                    (true, false) => 0x42,
                    (false, false) => 0x43,
                });
                out.put_u16(*id);
                matrix.write(out);
            }
        }
    }
}

/// What a line is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub enum LinePaint {
    Solid(Rgba8),
    Fill(FillStyle),
}

/// A `LINESTYLE2`.
#[derive(Debug, Clone, PartialEq)]
#[expect(clippy::struct_excessive_bools, reason = "the fields mirror the flag bits of the SWF record")]
pub struct LineStyle {
    /// Width in twips; 0 is a hairline.
    pub width: u16,
    pub paint: LinePaint,
    pub start_cap: Cap,
    pub end_cap: Cap,
    pub join: Join,
    /// Ratio of miter length to half the stroke width (at least 1.0); only written for miter joins.
    pub miter_limit: f64,
    pub no_h_scale: bool,
    pub no_v_scale: bool,
    pub pixel_hinting: bool,
    /// Leave closed paths open (always draw caps).
    pub no_close: bool,
}

impl LineStyle {
    /// A scaling, round-capped, round-joined solid line.
    #[must_use]
    pub const fn solid(width: u16, color: Rgba8) -> Self {
        Self {
            width,
            paint: LinePaint::Solid(color),
            start_cap: Cap::Round,
            end_cap: Cap::Round,
            join: Join::Round,
            miter_limit: 3.0,
            no_h_scale: false,
            no_v_scale: false,
            pixel_hinting: false,
            no_close: false,
        }
    }

    /// Appends the `LINESTYLE2` record.
    pub fn write(&self, out: &mut Vec<u8>) {
        out.put_u16(self.width);
        self.write_flags(out);
        match &self.paint {
            LinePaint::Solid(color) => put_rgba(out, *color),
            LinePaint::Fill(fill) => fill.write(out),
        }
    }

    /// The two flag bytes (caps, join, scaling, hinting, close) and, for mitered joins, the miter limit: the part
    /// of a `LINESTYLE2` between the width and the paint.
    pub(crate) fn write_flags(&self, out: &mut Vec<u8>) {
        let cap = |c: Cap| match c {
            Cap::Round => 0_u8,
            Cap::None => 1,
            Cap::Square => 2,
        };
        let join = match self.join {
            Join::Round => 0_u8,
            Join::Bevel => 1,
            Join::Miter => 2,
        };
        let has_fill = matches!(self.paint, LinePaint::Fill(_));
        out.put_u8(
            cap(self.start_cap).wrapping_shl(6)
                | join.wrapping_shl(4)
                | u8::from(has_fill).wrapping_shl(3)
                | u8::from(self.no_h_scale).wrapping_shl(2)
                | u8::from(self.no_v_scale).wrapping_shl(1)
                | u8::from(self.pixel_hinting),
        );
        out.put_u8(u8::from(self.no_close).wrapping_shl(2) | cap(self.end_cap));
        if join == 2 {
            out.put_u16(u16::try_from(num::round_f64_to_i32(self.miter_limit.max(1.0) * 256.0).clamp(256, i32::from(u16::MAX))).unwrap_or(768));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stop(offset: f32, r: u8) -> GradientStop {
        GradientStop { offset, color: Rgba8::opaque(r, 0, 0) }
    }

    #[test]
    fn few_stops_pass_through_and_degenerate_inputs_are_padded() {
        let (g, reduced) = Gradient::from_stops(&[stop(1.0, 255), stop(0.0, 0)], Spread::Pad, Interpolation::Rgb);
        assert!(!reduced);
        assert_eq!(g.records.iter().map(|r| r.ratio).collect::<Vec<_>>(), vec![0, 255]);
        assert_eq!(g.records[0].color.r, 0);

        let (g, _) = Gradient::from_stops(&[], Spread::Pad, Interpolation::Rgb);
        assert_eq!(g.records.len(), 2);
        let (g, _) = Gradient::from_stops(&[stop(0.5, 9)], Spread::Pad, Interpolation::Rgb);
        assert_eq!(g.records.len(), 2);
        assert_eq!(g.records[1].color.r, 9);
    }

    #[test]
    fn many_stops_are_reduced_keeping_ends_and_the_odd_one_out() {
        // 40 stops on a straight ramp with one spike in the middle
        let mut stops: Vec<GradientStop> = (0..40_u8).map(|i| stop(f32::from(i) / 39.0, i.saturating_mul(6))).collect();
        stops[20].color = Rgba8::opaque(255, 255, 0);
        let (g, reduced) = Gradient::from_stops(&stops, Spread::Reflect, Interpolation::LinearRgb);
        assert!(reduced);
        assert_eq!(g.records.len(), MAX_GRADIENT_RECORDS);
        assert_eq!(g.records.first().unwrap().ratio, 0);
        assert_eq!(g.records.last().unwrap().ratio, 255);
        assert!(g.records.iter().any(|r| r.color.g == 255), "the spike survives");
        assert!(g.records.windows(2).all(|w| w[0].ratio <= w[1].ratio));
    }

    #[test]
    fn hostile_stop_counts_and_nan_do_not_panic() {
        let stops: Vec<GradientStop> = (0..5000).map(|i| stop(if i % 7 == 0 { f32::NAN } else { 0.5 }, 1)).collect();
        let (g, reduced) = Gradient::from_stops(&stops, Spread::Pad, Interpolation::Rgb);
        assert!(reduced);
        assert_eq!(g.records.len(), MAX_GRADIENT_RECORDS);
    }

    #[test]
    fn fill_style_bytes() {
        let mut out = Vec::new();
        FillStyle::Solid(Rgba8::new(1, 2, 3, 4)).write(&mut out);
        assert_eq!(out, vec![0x00, 1, 2, 3, 4]);

        let mut out = Vec::new();
        FillStyle::Bitmap { id: 7, matrix: Matrix::IDENTITY, repeat: false, smoothed: true }.write(&mut out);
        assert_eq!(&out[..3], &[0x41, 7, 0]);
        let mut out = Vec::new();
        FillStyle::Bitmap { id: 7, matrix: Matrix::IDENTITY, repeat: true, smoothed: false }.write(&mut out);
        assert_eq!(out[0], 0x42);
    }

    #[test]
    fn line_style2_layout() {
        let mut out = Vec::new();
        let mut line = LineStyle::solid(40, Rgba8::opaque(9, 8, 7));
        line.start_cap = Cap::None;
        line.end_cap = Cap::Square;
        line.join = Join::Miter;
        line.miter_limit = 2.0;
        line.no_h_scale = true;
        line.write(&mut out);
        // width, start cap none (01) join miter (10) fill 0 noH 1 noV 0 hint 0, noclose 0 end cap square (10)
        assert_eq!(out, vec![40, 0, 0b0110_0100, 0b0000_0010, 0, 2, 9, 8, 7, 255]);
    }
}
