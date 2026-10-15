//! Native SWF shapes from Animate's edge shapes.
//!
//! An Animate `DOMShape` ([`anim_shape::EdgeShape`]) is stored the way SWF stores shapes: runs of lines and curves
//! that say which fill lies on their left and right and whether they are stroked, with 1-based style indices. So
//! the conversion is nearly a copy: styles map one to one, and every edge becomes a run of edge records with the
//! same `fill0`, `fill1` and line style. Nothing is assembled into closed outlines first, which keeps shared
//! boundaries shared (and avoids the hairline gaps that separate fills would show).
//!
//! Cubic segments (newer Animate files carry the exact curves next to their quadratic approximation) are converted
//! to quadratics, the only curves SWF has. Patterned strokes (dashed, dotted, ragged, stipple, hatched) become their
//! solid base, as in the display lists.

use anim_core::kurbo::PathEl;
use anim_core::{Point, Rgba8};
use anim_display::FillRule;
use anim_shape::display::display_stroke;
use anim_shape::{EdgeShape, FillStyle as EdgeFill, GradientFill, Segment, StrokeStyle as EdgeStroke, TWIPS_PER_PX};

use crate::error::SwfError;
use crate::export::{ExportNote, line_style};
use crate::shape::{PathStyle, ShapeBuilder, ShapeDef};
use crate::style::{FillStyle, Gradient, LinePaint};
use crate::types::{CharacterId, Matrix};

/// What a bitmap fill that cannot be resolved is painted with: mid grey, so that the shape stays visible.
pub const MISSING_BITMAP_COLOR: Rgba8 = Rgba8::opaque(128, 128, 128);

/// Segment ends closer than this (in twips) count as the same point.
const JOIN_EPSILON: f64 = 1.0e-6;

/// Finds the bitmap character behind a bitmap fill's library path.
pub trait BitmapCharacters {
    /// The character for the library item at `path`, or `None` when it has not been defined.
    fn character(&self, path: &str) -> Option<CharacterId>;
}

impl<F: Fn(&str) -> Option<CharacterId>> BitmapCharacters for F {
    fn character(&self, path: &str) -> Option<CharacterId> {
        self(path)
    }
}

/// A resolver that knows no bitmaps (bitmap fills turn grey).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBitmaps;

impl BitmapCharacters for NoBitmaps {
    fn character(&self, _path: &str) -> Option<CharacterId> {
        None
    }
}

/// A converted shape and what could not be carried over exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeConversion {
    pub shape: ShapeDef,
    pub notes: Vec<ExportNote>,
}

/// Converts an edge shape into a SWF shape (see the [module documentation](self)).
///
/// # Errors
/// [`SwfError::TooManyStyles`] for shapes with more styles than a SWF shape can address.
pub fn shape_from_edges(shape: &EdgeShape, bitmaps: &dyn BitmapCharacters) -> Result<EdgeConversion, SwfError> {
    let mut notes = Vec::new();
    let mut builder = ShapeBuilder::new();
    // Planar shapes have no overlaps, so the rule does not matter; even-odd is the SWF default.
    builder.set_fill_rule(FillRule::EvenOdd);
    for fill in &shape.fills {
        builder.add_fill(fill_style(fill, bitmaps, &mut notes))?;
    }
    for stroke in &shape.strokes {
        builder.add_line(stroke_style(stroke, bitmaps, &mut notes))?;
    }
    let valid = |index: u32, count: usize| if usize::try_from(index).is_ok_and(|i| i <= count) { index } else { 0 };
    for edge in &shape.edges {
        let style = PathStyle {
            fill0: valid(edge.fill_style0, shape.fills.len()),
            fill1: valid(edge.fill_style1, shape.fills.len()),
            line: valid(edge.stroke_style, shape.strokes.len()),
        };
        builder.add_edge_elements(&path_elements(&edge.segments), style);
    }
    Ok(EdgeConversion { shape: builder.finish(), notes })
}

fn note_once(notes: &mut Vec<ExportNote>, note: ExportNote) {
    if !notes.contains(&note) {
        notes.push(note);
    }
}

/// The segments of an edge (twips) as path elements in pixels; a gap between segments starts a new run.
fn path_elements(segments: &[Segment]) -> Vec<PathEl> {
    let px = |p: Point| Point::new(p.x / TWIPS_PER_PX, p.y / TWIPS_PER_PX);
    let mut elements = Vec::with_capacity(segments.len().saturating_add(1));
    let mut pen: Option<Point> = None;
    for segment in segments {
        if pen.is_none_or(|p| p.distance(segment.start()) > JOIN_EPSILON) {
            elements.push(PathEl::MoveTo(px(segment.start())));
        }
        elements.push(match *segment {
            Segment::Line { to, .. } => PathEl::LineTo(px(to)),
            Segment::Quad { ctrl, to, .. } => PathEl::QuadTo(px(ctrl), px(to)),
            Segment::Cubic { ctrl1, ctrl2, to, .. } => PathEl::CurveTo(px(ctrl1), px(ctrl2), px(to)),
        });
        pen = Some(segment.end());
    }
    elements
}

fn gradient_parts(gradient: &GradientFill, notes: &mut Vec<ExportNote>) -> (Matrix, Gradient) {
    let (swf, reduced) = Gradient::from_stops(&gradient.stops, gradient.spread, gradient.interpolation);
    if reduced {
        note_once(notes, ExportNote::GradientStopsReduced);
    }
    // XFL stores gradient matrices from gradient space into pixels; SWF's go into twips.
    (Matrix::from_affine(gradient.matrix), swf)
}

fn fill_style(fill: &EdgeFill, bitmaps: &dyn BitmapCharacters, notes: &mut Vec<ExportNote>) -> FillStyle {
    match fill {
        EdgeFill::Solid(color) => FillStyle::Solid(*color),
        EdgeFill::LinearGradient(g) => {
            let (matrix, gradient) = gradient_parts(g, notes);
            FillStyle::Linear { matrix, gradient }
        }
        EdgeFill::RadialGradient(g) => {
            let (matrix, gradient) = gradient_parts(g, notes);
            if g.focal.abs() > 1e-4 { FillStyle::Focal { matrix, gradient, focal: g.focal.clamp(-1.0, 1.0) } } else { FillStyle::Radial { matrix, gradient } }
        }
        EdgeFill::Bitmap(b) => {
            let Some(id) = bitmaps.character(&b.path) else {
                note_once(notes, ExportNote::MissingBitmap);
                return FillStyle::Solid(MISSING_BITMAP_COLOR);
            };
            // The stored bitmap matrix has SWF's linear part (a bitmap at 100% has scale 20) and its translation in pixels.
            FillStyle::Bitmap { id: id.0, matrix: Matrix::from_affine(b.matrix), repeat: !b.clipped, smoothed: b.smoothing }
        }
    }
}

fn stroke_style(stroke: &EdgeStroke, bitmaps: &dyn BitmapCharacters, notes: &mut Vec<ExportNote>) -> crate::style::LineStyle {
    let solid = stroke.solid();
    let paint = match &solid.fill {
        EdgeFill::Solid(color) => LinePaint::Solid(*color),
        other => LinePaint::Fill(fill_style(other, bitmaps, notes)),
    };
    let mut line = line_style(&display_stroke(solid), paint);
    line.pixel_hinting = solid.pixel_hinting;
    line
}

#[cfg(test)]
mod tests {
    use anim_core::Affine;
    use anim_display::{Cap, Interpolation, Join, Spread, StrokeScale};
    use anim_shape::{BitmapFill, Edge, SolidStroke};

    use super::*;
    use crate::shape::ShapeRecord;

    fn line(from: (f64, f64), to: (f64, f64)) -> Segment {
        Segment::Line { from: Point::new(from.0, from.1), to: Point::new(to.0, to.1) }
    }

    #[test]
    fn styles_map_one_to_one_and_edges_keep_their_sides() {
        let shape = EdgeShape {
            fills: vec![EdgeFill::Solid(Rgba8::opaque(255, 0, 0)), EdgeFill::Solid(Rgba8::opaque(0, 0, 255))],
            strokes: vec![EdgeStroke::plain(2.0, Rgba8::BLACK)],
            edges: vec![
                Edge::new(0, 1, 0, vec![line((0.0, 0.0), (400.0, 0.0)), line((400.0, 0.0), (400.0, 400.0))]),
                Edge::new(2, 1, 1, vec![line((400.0, 400.0), (0.0, 400.0))]),
                // an edge pointing at styles that do not exist draws without them
                Edge::new(9, 9, 9, vec![line((0.0, 400.0), (0.0, 0.0))]),
            ],
        };
        let out = shape_from_edges(&shape, &NoBitmaps).unwrap();
        assert_eq!(out.notes, vec![]);
        assert_eq!((out.shape.fills.len(), out.shape.lines.len()), (2, 1));
        assert!(!out.shape.non_zero_winding);
        assert_eq!(out.shape.lines[0].width, 40);
        let styles: Vec<_> =
            out.shape.records.iter().filter_map(|r| if let ShapeRecord::Style(s) = r { Some((s.fill0, s.fill1, s.line)) } else { None }).collect();
        // first edge: fill 1 on the right; second: both sides and the line; third: all cleared again
        assert_eq!(styles, vec![(None, Some(1), None), (Some(2), None, Some(1)), (Some(0), Some(0), Some(0))]);
        // edges that continue from the previous end need no move; a gap gets one
        let moves = out.shape.records.iter().filter(|r| matches!(r, ShapeRecord::Style(s) if s.move_to.is_some())).count();
        assert_eq!(moves, 3);
        out.shape.encode().unwrap();
    }

    #[test]
    fn runs_are_not_closed_and_gaps_start_new_runs() {
        let shape = EdgeShape {
            fills: vec![EdgeFill::Solid(Rgba8::BLACK)],
            strokes: vec![],
            edges: vec![Edge::new(0, 1, 0, vec![line((0.0, 0.0), (100.0, 0.0)), line((100.0, 0.0), (100.0, 100.0)), line((500.0, 500.0), (600.0, 500.0))])],
        };
        let out = shape_from_edges(&shape, &NoBitmaps).unwrap();
        let kinds: Vec<_> = out.shape.records.iter().map(|r| matches!(r, ShapeRecord::Style(_))).collect();
        // move, line, line, move, line: no closing edge was added
        assert_eq!(kinds, vec![true, false, false, true, false]);
    }

    #[test]
    fn cubics_become_quadratics_and_stay_on_the_end_point() {
        let shape = EdgeShape {
            fills: vec![EdgeFill::Solid(Rgba8::BLACK)],
            strokes: vec![],
            edges: vec![Edge::new(
                0,
                1,
                0,
                vec![Segment::Cubic {
                    from: Point::new(0.0, 0.0),
                    ctrl1: Point::new(0.0, 1000.0),
                    ctrl2: Point::new(2000.0, 1000.0),
                    to: Point::new(2000.0, 0.0),
                }],
            )],
        };
        let out = shape_from_edges(&shape, &NoBitmaps).unwrap();
        let (mut x, mut y, mut curves) = (0_i32, 0_i32, 0);
        for record in &out.shape.records {
            if let ShapeRecord::Curve { control_dx, control_dy, anchor_dx, anchor_dy } = *record {
                x += control_dx + anchor_dx;
                y += control_dy + anchor_dy;
                curves += 1;
            }
        }
        assert!(curves >= 2);
        assert_eq!((x, y), (2000, 0));
    }

    #[test]
    fn gradients_bitmaps_and_patterned_strokes() {
        let gradient = GradientFill {
            matrix: Affine::new([0.5, 0.0, 0.0, 0.5, 10.0, 20.0]),
            stops: vec![anim_display::GradientStop { offset: 0.0, color: Rgba8::WHITE }, anim_display::GradientStop { offset: 1.0, color: Rgba8::BLACK }],
            spread: Spread::Repeat,
            interpolation: Interpolation::LinearRgb,
            focal: 0.5,
        };
        let bitmap = BitmapFill { path: "pic.png".into(), matrix: Affine::new([20.0, 0.0, 0.0, 20.0, 3.0, 4.0]), clipped: true, smoothing: false };
        let patterned = EdgeStroke::Other(anim_shape::OtherStroke {
            kind: anim_shape::StrokeKind::Dashed,
            base: SolidStroke { weight: 0.0, caps: Cap::Square, joints: Join::Miter, scale_mode: StrokeScale::Horizontal, ..SolidStroke::default() },
            attrs: vec![],
        });
        let shape = EdgeShape {
            fills: vec![
                EdgeFill::RadialGradient(gradient),
                EdgeFill::Bitmap(bitmap.clone()),
                EdgeFill::Bitmap(BitmapFill { path: "gone.png".into(), ..bitmap }),
            ],
            strokes: vec![patterned],
            edges: vec![],
        };
        let resolver = |path: &str| (path == "pic.png").then_some(CharacterId(7));
        let out = shape_from_edges(&shape, &resolver).unwrap();
        assert_eq!(out.notes, vec![ExportNote::MissingBitmap]);
        let FillStyle::Focal { matrix, focal, .. } = &out.shape.fills[0] else { panic!("focal gradient") };
        assert_eq!((matrix.tx, matrix.ty), (200, 400));
        assert!((focal - 0.5).abs() < 1e-9);
        let FillStyle::Bitmap { id, matrix, repeat, smoothed } = &out.shape.fills[1] else { panic!("bitmap") };
        assert_eq!((*id, matrix.scale_x, matrix.tx, matrix.ty, *repeat, *smoothed), (7, 20 * crate::types::FIXED_ONE, 60, 80, false, false));
        assert_eq!(out.shape.fills[2], FillStyle::Solid(MISSING_BITMAP_COLOR));
        // a weight of zero is a hairline: fixed width in both directions whatever the scale mode says
        let line = &out.shape.lines[0];
        assert_eq!(line.width, 0);
        assert!(line.no_h_scale && line.no_v_scale);
        assert_eq!((line.start_cap, line.join), (Cap::Square, Join::Miter));
    }
}
