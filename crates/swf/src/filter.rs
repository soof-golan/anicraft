//! `FILTERLIST` for `PlaceObject3`.

use anim_display::Filter;

use crate::bits::PutBytes;
use crate::error::SwfError;
use crate::types::{put_rgba, to_fixed8, to_fixed16};

/// Most filters one object can carry (the count is a byte).
pub const MAX_FILTERS: usize = 255;

/// Appends the `FILTERLIST`: a count byte and the filters.
///
/// Quality maps to the number of blur passes. Display-list drop shadows and glows carry no quality, so they use one
/// pass.
///
/// # Errors
/// [`SwfError::TooManyFilters`] for more than [`MAX_FILTERS`] filters.
pub fn write_filter_list(out: &mut Vec<u8>, filters: &[Filter]) -> Result<(), SwfError> {
    if filters.len() > MAX_FILTERS {
        return Err(SwfError::TooManyFilters(filters.len()));
    }
    out.put_u8(u8::try_from(filters.len()).unwrap_or(u8::MAX));
    for filter in filters {
        match filter {
            Filter::DropShadow { color, angle_deg, distance, blur_x, blur_y, strength, inner, knockout, hide_object } => {
                out.put_u8(0);
                put_rgba(out, *color);
                out.put_i32(to_fixed16(*blur_x));
                out.put_i32(to_fixed16(*blur_y));
                out.put_i32(to_fixed16(angle_deg.to_radians()));
                out.put_i32(to_fixed16(*distance));
                out.put_i16(to_fixed8(*strength));
                out.put_u8(flags(*inner, *knockout, !*hide_object, 1));
            }
            Filter::Blur { x, y, quality } => {
                out.put_u8(1);
                out.put_i32(to_fixed16(*x));
                out.put_i32(to_fixed16(*y));
                out.put_u8(passes(*quality).wrapping_shl(3));
            }
            Filter::Glow { color, blur_x, blur_y, strength, inner, knockout } => {
                out.put_u8(2);
                put_rgba(out, *color);
                out.put_i32(to_fixed16(*blur_x));
                out.put_i32(to_fixed16(*blur_y));
                out.put_i16(to_fixed8(*strength));
                out.put_u8(flags(*inner, *knockout, true, 1));
            }
            Filter::ColorMatrix(matrix) => {
                out.put_u8(6);
                for v in matrix {
                    out.put_f32(*v);
                }
            }
        }
    }
    Ok(())
}

/// Blur passes from a 1 ..= 3 quality setting (anything else is clamped into 1 ..= 15).
fn passes(quality: u8) -> u8 {
    quality.clamp(1, 15)
}

/// The shared flag byte: inner, knockout, composite source and a five bit pass count.
fn flags(inner: bool, knockout: bool, composite: bool, passes: u8) -> u8 {
    u8::from(inner).wrapping_shl(7) | u8::from(knockout).wrapping_shl(6) | u8::from(composite).wrapping_shl(5) | (passes & 0x1f)
}

#[cfg(test)]
mod tests {
    use anim_core::Rgba8;

    use super::*;

    #[test]
    fn blur_layout() {
        let mut out = Vec::new();
        write_filter_list(&mut out, &[Filter::Blur { x: 5.0, y: 2.5, quality: 2 }]).unwrap();
        assert_eq!(out[0], 1);
        assert_eq!(out[1], 1);
        assert_eq!(&out[2..6], &(5 * 65536_i32).to_le_bytes());
        assert_eq!(&out[6..10], &(5 * 65536_i32 / 2).to_le_bytes());
        assert_eq!(out[10], 2 << 3);
        assert_eq!(out.len(), 11);
    }

    #[test]
    fn drop_shadow_layout() {
        let mut out = Vec::new();
        let filter = Filter::DropShadow {
            color: Rgba8::new(1, 2, 3, 4),
            angle_deg: 90.0,
            distance: 4.0,
            blur_x: 6.0,
            blur_y: 8.0,
            strength: 1.0,
            inner: true,
            knockout: false,
            hide_object: false,
        };
        write_filter_list(&mut out, &[filter]).unwrap();
        // count, id, rgba, 4 fixed, strength, flags
        assert_eq!(out.len(), 1 + 1 + 4 + 16 + 2 + 1);
        assert_eq!(&out[2..6], &[1, 2, 3, 4]);
        assert_eq!(&out[14..18], &to_fixed16(std::f64::consts::FRAC_PI_2).to_le_bytes());
        assert_eq!(&out[22..24], &256_i16.to_le_bytes());
        assert_eq!(out[24], 0b1010_0001);
    }

    #[test]
    fn color_matrix_is_twenty_floats() {
        let mut out = Vec::new();
        let mut m = [0.0_f32; 20];
        m[0] = 1.0;
        write_filter_list(&mut out, &[Filter::ColorMatrix(m)]).unwrap();
        assert_eq!(out.len(), 2 + 80);
        assert_eq!(out[1], 6);
    }

    #[test]
    fn too_many_filters_are_rejected() {
        let filters = vec![Filter::Blur { x: 1.0, y: 1.0, quality: 1 }; 256];
        assert!(write_filter_list(&mut Vec::new(), &filters).is_err());
    }
}
