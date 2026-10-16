//! The bit-packed SWF value types: `RECT`, `MATRIX`, `CXFORMWITHALPHA`, colours and twips.
//!
//! One pixel is 20 twips. Everything a SWF shape or placement stores is an integer number of twips (or a 16.16 /
//! 8.8 fixed-point number for scales, skews and colour factors), so the conversions from the `f64` pixel world of
//! [`anim_core`] live here and saturate instead of overflowing.

use anim_core::{Affine, ColorTransform, Rgba8, num};

use crate::bits::{BitWriter, PutBytes, sb_bits, sb_bits_all};

/// Twips per pixel.
pub const TWIPS_PER_PX: f64 = 20.0;

/// Largest coordinate magnitude we write, in twips (about 3.3 million pixels). Keeps every delta between two
/// coordinates inside 28 bits and bounds the work of splitting long edges.
pub const MAX_TWIPS: i32 = 1 << 26;

/// 16.16 fixed point one.
pub const FIXED_ONE: i32 = 1 << 16;

/// Largest 16.16 value we write (keeps the field at 31 bits).
const MAX_FIXED: i32 = (1 << 30) - 1;

/// Pixels to twips, rounded and clamped to ±[`MAX_TWIPS`]. `NaN` becomes 0.
#[must_use]
pub fn to_twips(px: f64) -> i32 {
    num::round_f64_to_i32(px * TWIPS_PER_PX).clamp(-MAX_TWIPS, MAX_TWIPS)
}

/// Twips back to pixels.
#[must_use]
pub fn from_twips(twips: i32) -> f64 {
    f64::from(twips) / TWIPS_PER_PX
}

/// A number as 16.16 fixed point, clamped so the bit field stays at most 31 bits wide. `NaN` becomes 0.
#[must_use]
pub fn to_fixed16(value: f64) -> i32 {
    num::round_f64_to_i32(value * f64::from(FIXED_ONE)).clamp(-MAX_FIXED, MAX_FIXED)
}

/// A number as 8.8 fixed point (`FIXED8`), saturating. `NaN` becomes 0.
#[must_use]
pub fn to_fixed8(value: f64) -> i16 {
    i16::try_from(num::round_f64_to_i32(value * 256.0).clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
}

/// A character id: the key of a shape, bitmap, sprite or sound in the SWF dictionary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharacterId(pub u16);

/// An axis-aligned rectangle in twips (`RECT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x_min: i32,
    pub x_max: i32,
    pub y_min: i32,
    pub y_max: i32,
}

impl Rect {
    #[must_use]
    pub const fn new(x_min: i32, x_max: i32, y_min: i32, y_max: i32) -> Self {
        Self { x_min, x_max, y_min, y_max }
    }

    /// A pixel rectangle converted to twips, widened outwards so that rounding never clips content.
    #[must_use]
    pub fn from_px(rect: anim_core::Rect) -> Self {
        let lo = |v: f64| num::floor_f64_to_i32(v * TWIPS_PER_PX).clamp(-MAX_TWIPS, MAX_TWIPS);
        let hi = |v: f64| num::ceil_f64_to_i32(v * TWIPS_PER_PX).clamp(-MAX_TWIPS, MAX_TWIPS);
        Self { x_min: lo(rect.min_x()), x_max: hi(rect.max_x()), y_min: lo(rect.min_y()), y_max: hi(rect.max_y()) }
    }

    /// The smallest rectangle containing both.
    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self { x_min: self.x_min.min(other.x_min), x_max: self.x_max.max(other.x_max), y_min: self.y_min.min(other.y_min), y_max: self.y_max.max(other.y_max) }
    }

    /// The rectangle grown by `by` twips on every side.
    #[must_use]
    pub fn inflate(self, by: i32) -> Self {
        Self {
            x_min: self.x_min.saturating_sub(by).max(-MAX_TWIPS),
            x_max: self.x_max.saturating_add(by).min(MAX_TWIPS),
            y_min: self.y_min.saturating_sub(by).max(-MAX_TWIPS),
            y_max: self.y_max.saturating_add(by).min(MAX_TWIPS),
        }
    }

    /// Appends the byte-aligned `RECT` encoding to `out`.
    pub fn write(&self, out: &mut Vec<u8>) {
        let mut w = BitWriter::appending_to(std::mem::take(out));
        let n = sb_bits_all(&[self.x_min, self.x_max, self.y_min, self.y_max]).min(31);
        w.ub(n, 5);
        for v in [self.x_min, self.x_max, self.y_min, self.y_max] {
            w.sb(v, n);
        }
        *out = w.into_bytes();
    }
}

/// A 2x3 transform as SWF stores it (`MATRIX`): 16.16 fixed scale and skew, translation in twips.
///
/// `x' = scale_x * x + skew1 * y + tx` and `y' = skew0 * x + scale_y * y + ty`, matching `kurbo::Affine` with
/// `[a, b, c, d, e, f] = [scale_x, skew0, skew1, scale_y, tx, ty]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Matrix {
    pub scale_x: i32,
    pub scale_y: i32,
    pub skew0: i32,
    pub skew1: i32,
    pub tx: i32,
    pub ty: i32,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Self = Self { scale_x: FIXED_ONE, scale_y: FIXED_ONE, skew0: 0, skew1: 0, tx: 0, ty: 0 };

    /// A matrix between two twip spaces from a pixel-space affine: the linear part is unchanged (a ratio does not
    /// depend on the unit) and the translation goes from pixels to twips. This is what placements and gradient
    /// matrices need (gradient space is measured in twips, ±16384).
    #[must_use]
    pub fn from_affine(affine: Affine) -> Self {
        let [scale_x, skew0, skew1, scale_y, tx, ty] = affine.as_coeffs();
        Self {
            scale_x: to_fixed16(scale_x),
            scale_y: to_fixed16(scale_y),
            skew0: to_fixed16(skew0),
            skew1: to_fixed16(skew1),
            tx: to_twips(tx),
            ty: to_twips(ty),
        }
    }

    /// Like [`Matrix::from_affine`] for a matrix that maps *bitmap pixels* (not twips) into twips, which is what
    /// bitmap fills need: the linear part is multiplied by 20 as well.
    #[must_use]
    pub fn from_bitmap_affine(affine: Affine) -> Self {
        // After `scale(20)` the translation is already in twips.
        let [scale_x, skew0, skew1, scale_y, tx, ty] = affine.as_coeffs().map(|v| v * TWIPS_PER_PX);
        let twips = |v: f64| num::round_f64_to_i32(v).clamp(-MAX_TWIPS, MAX_TWIPS);
        Self { scale_x: to_fixed16(scale_x), scale_y: to_fixed16(scale_y), skew0: to_fixed16(skew0), skew1: to_fixed16(skew1), tx: twips(tx), ty: twips(ty) }
    }

    /// The pixel-space affine this matrix stands for (the inverse of [`Matrix::from_affine`]).
    #[must_use]
    pub fn to_affine(&self) -> Affine {
        let fixed = |v: i32| f64::from(v) / f64::from(FIXED_ONE);
        Affine::new([fixed(self.scale_x), fixed(self.skew0), fixed(self.skew1), fixed(self.scale_y), from_twips(self.tx), from_twips(self.ty)])
    }

    /// Appends the byte-aligned `MATRIX` encoding to `out`.
    pub fn write(&self, out: &mut Vec<u8>) {
        let mut w = BitWriter::appending_to(std::mem::take(out));
        self.write_bits(&mut w);
        *out = w.into_bytes();
    }

    /// Writes the `MATRIX` into a bit stream without aligning (fill styles align afterwards).
    pub fn write_bits(&self, w: &mut BitWriter) {
        let has_scale = self.scale_x != FIXED_ONE || self.scale_y != FIXED_ONE;
        w.bit(has_scale);
        if has_scale {
            let n = sb_bits_all(&[self.scale_x, self.scale_y]).min(31);
            w.ub(n, 5);
            w.sb(self.scale_x, n);
            w.sb(self.scale_y, n);
        }
        let has_rotate = self.skew0 != 0 || self.skew1 != 0;
        w.bit(has_rotate);
        if has_rotate {
            let n = sb_bits_all(&[self.skew0, self.skew1]).min(31);
            w.ub(n, 5);
            w.sb(self.skew0, n);
            w.sb(self.skew1, n);
        }
        let n = sb_bits(self.tx).max(sb_bits(self.ty)).min(31);
        w.ub(n, 5);
        w.sb(self.tx, n);
        w.sb(self.ty, n);
    }
}

/// A colour transform as SWF stores it (`CXFORMWITHALPHA`): multipliers in 8.8 (256 = 1.0) and additive terms in
/// 0 ..= 255 units, per channel in r, g, b, a order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CxForm {
    pub mult: [i32; 4],
    pub add: [i32; 4],
}

impl Default for CxForm {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Largest magnitude of a colour transform term (the field is at most 15 bits wide).
const MAX_CX_TERM: i32 = (1 << 14) - 1;

impl CxForm {
    pub const IDENTITY: Self = Self { mult: [256; 4], add: [0; 4] };

    /// Converts an `anim_core` transform (unit-space offsets) to SWF units.
    #[must_use]
    pub fn from_color_transform(ct: &ColorTransform) -> Self {
        let mut mult = [256; 4];
        let mut add = [0; 4];
        for (m, src) in mult.iter_mut().zip(ct.mul) {
            *m = num::round_f64_to_i32(f64::from(src) * 256.0).clamp(-MAX_CX_TERM, MAX_CX_TERM);
        }
        for (a, src) in add.iter_mut().zip(ct.add) {
            *a = num::round_f64_to_i32(f64::from(src) * 255.0).clamp(-255, 255);
        }
        Self { mult, add }
    }

    #[must_use]
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    /// Appends the byte-aligned `CXFORMWITHALPHA` encoding to `out`.
    pub fn write(&self, out: &mut Vec<u8>) {
        let has_mult = self.mult != [256; 4];
        let has_add = self.add != [0; 4];
        let mut n = 1;
        if has_mult {
            n = n.max(sb_bits_all(&self.mult));
        }
        if has_add {
            n = n.max(sb_bits_all(&self.add));
        }
        let n = n.min(15);
        let mut w = BitWriter::appending_to(std::mem::take(out));
        w.bit(has_add);
        w.bit(has_mult);
        w.ub(n, 4);
        if has_mult {
            for v in self.mult {
                w.sb(v, n);
            }
        }
        if has_add {
            for v in self.add {
                w.sb(v, n);
            }
        }
        *out = w.into_bytes();
    }
}

/// `RGB`: three bytes.
pub fn put_rgb(out: &mut Vec<u8>, c: Rgba8) {
    out.put_u8(c.r);
    out.put_u8(c.g);
    out.put_u8(c.b);
}

/// `RGBA`: four bytes, straight alpha (as `DefineShape3` and later store it).
pub fn put_rgba(out: &mut Vec<u8>, c: Rgba8) {
    put_rgb(out, c);
    out.put_u8(c.a);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twips_round_and_saturate() {
        assert_eq!(to_twips(1.0), 20);
        assert_eq!(to_twips(-0.025), -1);
        assert_eq!(to_twips(f64::NAN), 0);
        assert_eq!(to_twips(1e30), MAX_TWIPS);
        assert_eq!(to_twips(-1e30), -MAX_TWIPS);
        assert!((from_twips(30) - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn rect_encoding_matches_a_hand_computed_example() {
        // 550x400 px stage: x 0..11000, y 0..8000 -> 15 value bits + sign = 15 bits (11000 < 2^14)
        let mut out = Vec::new();
        Rect::new(0, 11000, 0, 8000).write(&mut out);
        // Nbits = 15 (01111), xmin = 0 (15 zeros), xmax = 11000, ymin = 0, ymax = 8000
        assert_eq!(out, vec![0x78, 0x00, 0x05, 0x5f, 0x00, 0x00, 0x0f, 0xa0, 0x00]);
    }

    #[test]
    fn matrix_identity_is_one_byte_of_flags_and_zero_translation() {
        let mut out = Vec::new();
        Matrix::IDENTITY.write(&mut out);
        // no scale, no rotate, nbits = 1 (00001), tx = 0, ty = 0 -> 0 0 00001 0 0 -> 0b0000_0010 0b0000_0000
        assert_eq!(out, vec![0b0000_0010, 0b0000_0000]);
    }

    #[test]
    fn matrix_from_affine_maps_back() {
        let a = Affine::new([2.0, 0.5, -0.25, 1.5, 10.0, -3.5]);
        let m = Matrix::from_affine(a);
        let back = m.to_affine();
        for (x, y) in a.as_coeffs().into_iter().zip(back.as_coeffs()) {
            assert!((x - y).abs() < 1e-4, "{x} vs {y}");
        }
        assert_eq!(m.tx, 200);
        assert_eq!(m.ty, -70);
    }

    #[test]
    fn bitmap_matrix_scales_the_linear_part_by_twips() {
        let m = Matrix::from_bitmap_affine(Affine::translate((5.0, 0.0)));
        assert_eq!(m.scale_x, 20 * FIXED_ONE);
        assert_eq!(m.scale_y, 20 * FIXED_ONE);
        assert_eq!(m.tx, 100);
        assert_eq!(m.ty, 0);
    }

    #[test]
    fn cxform_identity_has_no_terms() {
        let mut out = Vec::new();
        CxForm::IDENTITY.write(&mut out);
        assert_eq!(out.len(), 1);
        assert!(CxForm::from_color_transform(&ColorTransform::IDENTITY).is_identity());
    }

    #[test]
    fn cxform_converts_units() {
        let cx = CxForm::from_color_transform(&ColorTransform { mul: [0.5, 1.0, 1.0, 0.25], add: [1.0, 0.0, -1.0, 0.0] });
        assert_eq!(cx.mult, [128, 256, 256, 64]);
        assert_eq!(cx.add, [255, 0, -255, 0]);
    }
}
