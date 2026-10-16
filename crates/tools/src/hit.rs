//! Hit testing and bounds of stage elements.
//!
//! Shapes are tested against their regions and strokes through `anim-shape`; symbol instances, bitmaps and text
//! fields through the [`BoundsProvider`]; groups through their members.

use anim_core::{Affine, Point, Rect};
use anim_scene::{Element, MAX_GROUP_DEPTH};
use anim_shape::Hit;

use crate::context::{BoundsProvider, ToolContext};
use crate::selection::ElementRef;

/// What part of an element was hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PickPart {
    /// A filled region of a shape (1-based fill style).
    Fill { style: u32 },
    /// A stroke of a shape (1-based stroke style) at this distance from its centre line.
    Stroke { style: u32, distance: f64 },
    /// The body of a symbol, bitmap, text field or group.
    Body,
}

/// The topmost element at a point.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub element: ElementRef,
    pub part: PickPart,
}

/// An element that can be picked, with the transform to the stage of its container.
#[derive(Debug, Clone)]
pub struct Candidate<'a> {
    pub element_ref: ElementRef,
    pub element: &'a Element,
    pub parent_to_stage: Affine,
}

/// The bounding box of `rect` after `matrix`, or `None` when it is not finite.
#[must_use]
pub fn transform_rect(matrix: Affine, rect: Rect) -> Option<Rect> {
    let r = matrix.transform_rect_bbox(rect);
    (r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()).then_some(r)
}

/// True when two rectangles share at least a point.
#[must_use]
pub fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Bounds of an element in its parent's coordinates (own matrix applied).
#[must_use]
pub fn bounds_in_parent(provider: &dyn BoundsProvider, element: &Element, depth: usize) -> Option<Rect> {
    match element {
        Element::Shape(s) => transform_rect(s.matrix, s.shape.bounds_with_strokes()?),
        Element::Group(g) => {
            if depth >= MAX_GROUP_DEPTH {
                return None;
            }
            let inner = g.members.iter().filter_map(|m| bounds_in_parent(provider, m, depth.saturating_add(1))).reduce(|a, b| a.union(b))?;
            transform_rect(g.matrix, inner)
        }
        Element::Unknown(_) => None,
        other => transform_rect(other.matrix()?, provider.local_bounds(other)?),
    }
}

/// Every pickable element in the current frame, topmost first.
///
/// With a group scope only the members of that group are returned; otherwise the elements of the keyframes that
/// cover the playhead on every editable layer (layer 0 is on top).
#[must_use]
pub fn candidates<'a>(ctx: &ToolContext<'a>) -> Vec<Candidate<'a>> {
    let mut out = Vec::new();
    if let Some(scope) = &ctx.selection.scope {
        if let Some(resolved) = ctx.resolve(scope)
            && let Element::Group(g) = resolved.element
        {
            let to_stage = resolved.parent_to_stage * g.matrix;
            for (i, element) in g.members.iter().enumerate().rev().filter(|(_, e)| !crate::lock::is_locked(e)) {
                out.push(Candidate { element_ref: scope.child(i), element, parent_to_stage: to_stage });
            }
        }
        return out;
    }
    let Some(timeline) = ctx.timeline() else { return out };
    for (li, layer) in timeline.layers.iter().enumerate() {
        if !ctx.layer_editable(li) {
            continue;
        }
        // Edit Multiple Frames: the keyframes anywhere in the range, the one under the playhead first.
        let frames: Vec<&anim_scene::Frame> = match ctx.edit_range {
            Some((lo, hi)) => {
                let mut all: Vec<&anim_scene::Frame> = layer.frames.iter().filter(|f| f.index <= hi && f.end() > lo).collect();
                all.sort_by_key(|f| (!f.covers(ctx.frame), f.index));
                all
            }
            None => layer.keyframe_at(ctx.frame).into_iter().collect(),
        };
        for frame in frames {
            for (i, element) in frame.elements.iter().enumerate().rev().filter(|(_, e)| !crate::lock::is_locked(e)) {
                out.push(Candidate { element_ref: ElementRef::top_level(li, frame.index, i), element, parent_to_stage: Affine::IDENTITY });
            }
        }
    }
    out
}

fn invertible(m: Affine) -> Option<Affine> {
    let det = m.determinant();
    (det.is_finite() && det.abs() > 1.0e-12).then(|| m.inverse())
}

/// Length scale of a matrix (square root of the absolute determinant).
pub(crate) fn scale_of(m: Affine) -> f64 {
    m.determinant().abs().sqrt()
}

fn hit_element(provider: &dyn BoundsProvider, element: &Element, p: Point, tol: f64, depth: usize) -> Option<PickPart> {
    match element {
        Element::Shape(s) => {
            let inv = invertible(s.matrix)?;
            let q = inv * p;
            let local_tol = tol * scale_of(inv);
            let bounds = s.shape.bounds_with_strokes()?;
            if !bounds.inflate(local_tol, local_tol).contains(q) {
                return None;
            }
            match s.shape.hit_test(q, local_tol)? {
                Hit::Stroke { style, distance } => Some(PickPart::Stroke { style, distance }),
                Hit::Fill { style } => Some(PickPart::Fill { style }),
            }
        }
        Element::Group(g) => {
            if depth >= MAX_GROUP_DEPTH {
                return None;
            }
            let inv = invertible(g.matrix)?;
            let q = inv * p;
            let local_tol = tol * scale_of(inv);
            g.members.iter().rev().any(|m| hit_element(provider, m, q, local_tol, depth.saturating_add(1)).is_some()).then_some(PickPart::Body)
        }
        Element::Unknown(_) => None,
        other => {
            let inv = invertible(other.matrix()?)?;
            let q = inv * p;
            let local_tol = tol * scale_of(inv);
            let bounds = provider.local_bounds(other)?;
            let b = bounds.inflate(local_tol, local_tol);
            (q.x >= b.x0 && q.x <= b.x1 && q.y >= b.y0 && q.y <= b.y1).then_some(PickPart::Body)
        }
    }
}

/// The topmost element under `p` (stage coordinates), within the context's hit tolerance.
#[must_use]
pub fn pick(ctx: &ToolContext<'_>, p: Point) -> Option<Pick> {
    let tol = ctx.tolerance();
    for c in candidates(ctx) {
        let Some(inv) = invertible(c.parent_to_stage) else { continue };
        let local = inv * p;
        let local_tol = tol * scale_of(inv);
        if let Some(part) = hit_element(ctx.bounds, c.element, local, local_tol, 0) {
            return Some(Pick { element: c.element_ref, part });
        }
    }
    None
}

/// The elements whose bounds touch `rect` (stage coordinates), topmost first.
#[must_use]
pub fn elements_in_rect(ctx: &ToolContext<'_>, rect: Rect) -> Vec<ElementRef> {
    candidates(ctx)
        .into_iter()
        .filter(|c| ctx.bounds_in_parent(c.element).and_then(|b| transform_rect(c.parent_to_stage, b)).is_some_and(|b| rects_overlap(b, rect)))
        .map(|c| c.element_ref)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_includes_touching_edges() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(rects_overlap(a, Rect::new(10.0, 10.0, 20.0, 20.0)));
        assert!(!rects_overlap(a, Rect::new(10.1, 0.0, 20.0, 5.0)));
    }
}
