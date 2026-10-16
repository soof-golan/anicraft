//! The Lasso tool's selection: cut merge-mode shapes along the lasso (or the magic wand) and select the pieces.
//!
//! Animate selects the part of a shape that lies inside the lasso, leaving the rest where it was. The model has
//! no partial selection, so the part becomes an element of its own, right after what is left of the shape. Objects
//! (symbols, groups, text, bitmaps, drawing objects) are selected whole when the lasso encloses their bounds.

use anim_core::kurbo::Shape as _;
use anim_core::{Affine, BezPath, Point, Rect};
use anim_scene::{Document, Element, LayerKind, TimelineId};
use anim_shape::FillStyle;

use crate::selection::{ElementRef, Selection};
use crate::shapeops::{self, Split};
use crate::state::EditorState;

/// Layers the lasso reaches: visible, unlocked, with drawable content.
fn editable_layers(doc: &Document, timeline: &TimelineId) -> Vec<usize> {
    let Some(tl) = doc.timeline(timeline) else { return Vec::new() };
    (0..tl.layers.len())
        .filter(|&i| {
            matches!(tl.resolved_kind(i), Some(LayerKind::Normal | LayerKind::Guided | LayerKind::Masked | LayerKind::Mask))
                && tl.is_effectively_visible(i)
                && !tl.is_effectively_locked(i)
        })
        .collect()
}

/// The closed path through the polygon corners.
#[must_use]
pub fn polygon_path(polygon: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    for (i, p) in polygon.iter().enumerate() {
        if i == 0 {
            path.move_to(*p);
        } else {
            path.line_to(*p);
        }
    }
    path.close_path();
    path
}

fn encloses(region: &BezPath, rect: Rect) -> bool {
    [Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)]
        .into_iter()
        .all(|p| region.winding(p) != 0)
}

/// Applies `cut` to every free merge-mode shape of the keyframe of `layer` at the playhead and returns the elements
/// to select. `cut` returns the split of one shape; other elements are offered to `whole`.
fn cut_layer(
    doc: &mut Document,
    timeline: &TimelineId,
    layer: usize,
    frame: u32,
    cut: &dyn Fn(&anim_scene::ShapeElement) -> Split,
    whole: &dyn Fn(&Document, &Element) -> bool,
) -> Vec<ElementRef> {
    let Some(keyframe) = doc.timeline(timeline).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).map(|f| f.index) else {
        return Vec::new();
    };
    let Some(elements) =
        doc.timeline(timeline).and_then(|t| t.layers.get(layer)).and_then(|l| l.frames.iter().find(|f| f.index == keyframe)).map(|f| f.elements.clone())
    else {
        return Vec::new();
    };
    let mut rebuilt: Vec<Element> = Vec::with_capacity(elements.len());
    let mut picks: Vec<usize> = Vec::new();
    let mut changed = false;
    for element in elements {
        match element {
            Element::Shape(s) if !s.is_drawing_object && s.primitive.is_none() && s.matrix == Affine::IDENTITY => {
                let split = cut(&s);
                match (split.inside, split.outside) {
                    (None, _) => rebuilt.push(Element::Shape(s)),
                    (Some(_), None) => {
                        picks.push(rebuilt.len());
                        rebuilt.push(Element::Shape(s));
                    }
                    (Some(inside), Some(outside)) => {
                        changed = true;
                        let mut rest = s.clone();
                        rest.shape = outside;
                        rebuilt.push(Element::Shape(rest));
                        let mut part = s;
                        part.shape = inside;
                        picks.push(rebuilt.len());
                        rebuilt.push(Element::Shape(part));
                    }
                }
            }
            other => {
                if whole(doc, &other) {
                    picks.push(rebuilt.len());
                }
                rebuilt.push(other);
            }
        }
    }
    if changed
        && let Some(frame) = doc.timeline_mut(timeline).and_then(|t| t.layers.get_mut(layer)).and_then(|l| l.frames.iter_mut().find(|f| f.index == keyframe))
    {
        frame.elements = rebuilt;
    }
    picks.into_iter().map(|i| ElementRef::top_level(layer, keyframe, i)).collect()
}

/// Selects what lies inside the polygon (stage pixels). Returns whether the document changed.
pub fn lasso_select(doc: &mut Document, state: &mut EditorState, polygon: &[Point], additive: bool) -> bool {
    let region = polygon_path(polygon);
    let before = doc.clone();
    let mut items = if additive { state.selection.items.clone() } else { Vec::new() };
    if polygon.len() >= 3 {
        let timeline = state.timeline.clone();
        for layer in editable_layers(doc, &timeline) {
            items.extend(cut_layer(doc, &timeline, layer, state.frame, &|s| shapeops::split_by_region(&s.shape, &region), &|d, e| {
                anim_scene::element_bounds(e, &d.library).is_some_and(|b| encloses(&region, b))
            }));
        }
    }
    items.dedup();
    state.selection = Selection { items, scope: None };
    *doc != before
}

/// Selects the fills similar in colour to the one at `point` on the topmost layer that has one.
pub fn wand_select(doc: &mut Document, state: &mut EditorState, point: Point, threshold: f64, additive: bool) -> bool {
    let before = doc.clone();
    let mut items = if additive { state.selection.items.clone() } else { Vec::new() };
    let timeline = state.timeline.clone();
    let frame = state.frame;
    for layer in editable_layers(doc, &timeline) {
        let wanted: Option<FillStyle> = doc
            .timeline(&timeline)
            .and_then(|t| t.layers.get(layer))
            .and_then(|l| l.keyframe_at(frame))
            .and_then(|k| k.elements.iter().rev().find_map(|e| if let Element::Shape(s) = e { shapeops::paint_at(&s.shape, point) } else { None }));
        let Some(wanted) = wanted else { continue };
        items.extend(cut_layer(doc, &timeline, layer, frame, &|s| shapeops::split_by_paint(&s.shape, &wanted, threshold), &|_, _| false));
        break;
    }
    items.dedup();
    state.selection = Selection { items, scope: None };
    *doc != before
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic, reason = "test code")]

    use anim_core::Rgba8;
    use anim_scene::ShapeElement;
    use anim_shape::{EdgeShape, PathInput};

    use super::*;
    use crate::command::ToolCommand;
    use crate::editor::Editor;

    fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    fn editor_with(shapes: &[(BezPath, Rgba8)]) -> Editor {
        let mut e = Editor::new(Document::new(550.0, 400.0, 24.0));
        for (path, colour) in shapes {
            let shape = EdgeShape::from_paths(&[PathInput::new(path.clone(), Some(FillStyle::Solid(*colour)), None)]);
            let timeline = e.state.timeline.clone();
            anim_scene::edit::add_element(&mut e.document, &timeline, 0, 0, Element::Shape(ShapeElement::new(shape))).unwrap();
        }
        e
    }

    fn elements(e: &Editor) -> &[Element] {
        &e.document.timeline(&e.state.timeline).unwrap().layers[0].frames[0].elements
    }

    #[test]
    fn a_lasso_cuts_the_part_inside_out_of_a_shape() {
        let mut e = editor_with(&[(rect_path(0.0, 0.0, 100.0, 100.0), Rgba8::opaque(255, 0, 0))]);
        let polygon = vec![Point::new(-10.0, -10.0), Point::new(50.0, -10.0), Point::new(50.0, 110.0), Point::new(-10.0, 110.0)];
        e.run_commands(&[ToolCommand::LassoSelect { polygon, additive: false }]).unwrap();
        assert_eq!(e.state.selection.items.len(), 1);
        assert_eq!(elements(&e).len(), 2, "the shape was cut in two");
        let Element::Shape(part) = &elements(&e)[e.state.selection.items[0].path[0]] else { panic!("not a shape") };
        let b = part.shape.bounds().unwrap();
        assert!((b.x1 - 50.0).abs() < 0.1 && b.x0.abs() < 0.1, "the selected part is the left half: {b:?}");
        assert!(e.undo(), "the cut is one undo step");
        assert_eq!(elements(&e).len(), 1);
    }

    #[test]
    fn a_lasso_around_everything_selects_without_cutting() {
        let mut e = editor_with(&[(rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::BLACK)]);
        let polygon = vec![Point::new(-5.0, -5.0), Point::new(50.0, -5.0), Point::new(50.0, 50.0), Point::new(-5.0, 50.0)];
        e.run_commands(&[ToolCommand::LassoSelect { polygon, additive: false }]).unwrap();
        assert_eq!(e.state.selection.items.len(), 1);
        assert_eq!(elements(&e).len(), 1);
    }

    #[test]
    fn the_wand_selects_every_fill_of_a_similar_colour() {
        let mut e = editor_with(&[
            (rect_path(0.0, 0.0, 10.0, 10.0), Rgba8::opaque(200, 0, 0)),
            (rect_path(20.0, 0.0, 30.0, 10.0), Rgba8::opaque(0, 0, 200)),
            (rect_path(40.0, 0.0, 50.0, 10.0), Rgba8::opaque(204, 4, 0)),
        ]);
        e.run_commands(&[ToolCommand::WandSelect { point: Point::new(5.0, 5.0), threshold: 10.0, additive: false }]).unwrap();
        assert_eq!(e.state.selection.items.len(), 2, "the two red-ish shapes, not the blue one");
        assert_eq!(elements(&e).len(), 3);
    }
}
