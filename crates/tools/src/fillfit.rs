//! Placing gradient and bitmap fills on the shape they paint.
//!
//! A gradient chosen in the toolbox has no place of its own; Animate fits it to the shape it is first painted on
//! (the gradient box is the shape's bounding box) unless the paint bucket's lock fill option is on.

use anim_core::{Affine, Rect};
use anim_shape::FillStyle;

/// Half the side of the square gradient space that XFL gradient matrices map.
pub const GRADIENT_HALF: f64 = 819.2;

/// The matrix that maps gradient space onto `bounds`.
#[must_use]
pub fn gradient_box(bounds: Rect) -> Affine {
    let c = bounds.center();
    Affine::translate((c.x, c.y)) * Affine::scale_non_uniform(bounds.width().max(1.0) / (GRADIENT_HALF * 2.0), bounds.height().max(1.0) / (GRADIENT_HALF * 2.0))
}

/// The bounds a gradient matrix covers, the inverse of [`gradient_box`] for matrices without rotation or skew.
#[must_use]
pub fn gradient_bounds(matrix: Affine) -> Rect {
    let co = matrix.as_coeffs();
    let half_w = co[0].hypot(co[1]) * GRADIENT_HALF;
    let half_h = co[2].hypot(co[3]) * GRADIENT_HALF;
    Rect::new(co[4] - half_w, co[5] - half_h, co[4] + half_w, co[5] + half_h)
}

/// `fill` placed on `bounds`: gradients fill the box, bitmaps start at its top left corner. Solid fills are
/// returned unchanged.
#[must_use]
pub fn fit_fill(fill: &FillStyle, bounds: Rect) -> FillStyle {
    match fill {
        FillStyle::Solid(_) => fill.clone(),
        FillStyle::LinearGradient(g) => {
            let mut g = g.clone();
            g.matrix = gradient_box(bounds);
            FillStyle::LinearGradient(g)
        }
        FillStyle::RadialGradient(g) => {
            let mut g = g.clone();
            g.matrix = gradient_box(bounds);
            FillStyle::RadialGradient(g)
        }
        FillStyle::Bitmap(b) => {
            let mut b = b.clone();
            let mut co = b.matrix.as_coeffs();
            co[4] = bounds.x0;
            co[5] = bounds.y0;
            b.matrix = Affine::new(co);
            FillStyle::Bitmap(b)
        }
    }
}

#[cfg(test)]
mod tests {
    use anim_core::{Point, Rgba8};
    use anim_display::{GradientStop, Interpolation, Spread};
    use anim_shape::GradientFill;

    use super::*;

    fn gradient() -> FillStyle {
        FillStyle::LinearGradient(GradientFill {
            matrix: Affine::IDENTITY,
            stops: vec![GradientStop { offset: 0.0, color: Rgba8::BLACK }, GradientStop { offset: 1.0, color: Rgba8::opaque(255, 255, 255) }],
            spread: Spread::Pad,
            interpolation: Interpolation::Rgb,
            focal: 0.0,
        })
    }

    #[test]
    fn a_gradient_fits_the_box_it_is_painted_on() {
        let FillStyle::LinearGradient(g) = fit_fill(&gradient(), Rect::new(100.0, 50.0, 300.0, 150.0)) else { panic!("gradient") };
        let left = g.matrix * Point::new(-GRADIENT_HALF, 0.0);
        let right = g.matrix * Point::new(GRADIENT_HALF, 0.0);
        assert!((left.x - 100.0).abs() < 1.0e-6 && (right.x - 300.0).abs() < 1.0e-6);
        assert!((left.y - 100.0).abs() < 1.0e-6);
        let back = gradient_bounds(g.matrix);
        assert!((back.x0 - 100.0).abs() < 1.0e-6 && (back.y1 - 150.0).abs() < 1.0e-6);
    }

    #[test]
    fn solid_fills_are_unchanged() {
        let solid = FillStyle::Solid(Rgba8::BLACK);
        assert_eq!(fit_fill(&solid, Rect::new(0.0, 0.0, 10.0, 10.0)), solid);
    }
}
