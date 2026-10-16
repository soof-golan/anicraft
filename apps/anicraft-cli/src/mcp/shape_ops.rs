//! Modify > Shape and Modify > Combine Objects: the commands.
//!
//! The geometry is in `anicraft_tools::modify`; here the selection is resolved, the shapes are replaced and the
//! selection is moved onto the result. Every command is one undo step (the session snapshots the document) and a
//! failure changes nothing.

use anicraft_tools::ElementRef;
use anicraft_tools::modify::{self, Body, CombineOp, CurveEdit};
use anim_core::{Affine, Point};
use anim_scene::{Document, Element, ShapeElement, TimelineId};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use anim_shape::EdgeShape;
use serde::Deserialize;
use serde_json::{Value, json};

use super::arrange_ops::{self, element_mut, has_selection, one_parent, select_paths};
use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

const NO_PARAMS: &str = r#"{"type":"object","additionalProperties":false,"properties":{}}"#;

/// Registers the commands of this module.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(all()).expect("shape command ids are unique");
}

/// The commands.
#[must_use]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("modify.combine_objects", "Combine Objects", combine_objects)
            .in_menu(&["Modify", "Combine Objects"])
            .enabled_when(has_selection)
            .describe("Combines the selected shapes (drawing objects or merge-mode shapes of one keyframe, bottom-most first). `op`: union (one shape from all of them, painted like the topmost), intersect (the area all of them cover), punch (the topmost shape is cut out of the ones below it and disappears) or crop (the ones below are cut down to the topmost shape, which disappears). Results are drawing objects.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["op"],"properties":{"op":{"enum":["union","intersect","punch","crop"]}}}"#),
        CommandSpec::new("modify.shape_curves", "Smooth, Straighten, Optimize", shape_curves)
            .in_menu(&["Modify", "Shape"])
            .enabled_when(has_selection)
            .describe("Edits the curves of the selected shapes. `op`: smooth (fewer anchors, softer handles), straighten (nearly straight segments become straight; freehand rectangles, ellipses and polygons become the clean shape) or optimize (refits the curves with fewer anchors). `amount` is 0..100 (default 50); `passes` repeats the operation (default 1). Returns the anchor counts before and after.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["op"],"properties":{"op":{"enum":["smooth","straighten","optimize"]},"amount":{"type":"number","minimum":0,"maximum":100},"passes":{"type":"integer","minimum":1,"maximum":10}}}"#),
        CommandSpec::new("modify.convert_lines_to_fills", "Convert Lines to Fills", lines_to_fills)
            .in_menu(&["Modify", "Shape"])
            .enabled_when(has_selection)
            .describe("Turns every stroke of the selected shapes into a filled area painted with the stroke's colour.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.expand_fill", "Expand Fill", expand_fill)
            .in_menu(&["Modify", "Shape"])
            .enabled_when(has_selection)
            .describe("Grows (or with `inset` shrinks) the fills of the selected shapes by `distance` pixels.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["distance"],"properties":{"distance":{"type":"number","exclusiveMinimum":0,"maximum":1000},"inset":{"type":"boolean","default":false}}}"#),
        CommandSpec::new("modify.soften_fill_edges", "Soften Fill Edges", soften_fill_edges)
            .in_menu(&["Modify", "Shape"])
            .enabled_when(has_selection)
            .describe("Fades the edges of the solid fills of the selected shapes over `distance` pixels in `steps` translucent bands, growing outwards (default) or with `inset` shrinking inwards.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["distance"],"properties":{"distance":{"type":"number","exclusiveMinimum":0,"maximum":1000},"steps":{"type":"integer","minimum":1,"maximum":64,"default":4},"inset":{"type":"boolean","default":false}}}"#),
    ]
}

/// The shapes among the selected objects: their references.
fn selected_shapes(doc: &mut AniDoc) -> Vec<ElementRef> {
    let id = doc.editor.state.timeline.clone();
    let items = doc.editor.state.selection.items.clone();
    items.into_iter().filter(|r| matches!(element_mut(doc.document_mut(), &id, r), Some(Element::Shape(_)))).collect()
}

fn shape_mut<'a>(document: &'a mut Document, id: &TimelineId, r: &ElementRef) -> Option<&'a mut ShapeElement> {
    match element_mut(document, id, r) {
        Some(Element::Shape(s)) => Some(s),
        _ => None,
    }
}

/// Replaces the geometry of every selected shape by `f(shape)`; the shape's matrix and kind are kept.
fn map_shapes(doc: &mut AniDoc, what: &str, mut f: impl FnMut(&EdgeShape) -> EdgeShape) -> Result<usize, CommandError> {
    let refs = selected_shapes(doc);
    if refs.is_empty() {
        return Err(CommandError::failed(format!("{what} needs a selected shape")));
    }
    let id = doc.editor.state.timeline.clone();
    let mut touched = 0_usize;
    for r in &refs {
        if let Some(s) = shape_mut(doc.document_mut(), &id, r) {
            s.shape = f(&s.shape);
            s.primitive = None;
            touched = touched.saturating_add(1);
        }
    }
    Ok(touched)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CombineParams {
    op: String,
}

fn combine_objects(doc: &mut AniDoc, params: &Value) -> Out {
    let p: CombineParams = parse_params(params)?;
    let op = match p.op.as_str() {
        "union" => CombineOp::Union,
        "intersect" => CombineOp::Intersect,
        "punch" => CombineOp::Punch,
        "crop" => CombineOp::Crop,
        other => return Err(CommandError::invalid_params(format!("unknown combine operation {other:?}"))),
    };
    let sel = one_parent(doc)?;
    if sel.indices.len() < 2 {
        return Err(CommandError::failed("Combine Objects needs two or more selected shapes"));
    }
    let id = doc.editor.state.timeline.clone();
    let siblings = arrange_ops::siblings_mut(doc.document_mut(), &id, &sel).ok_or_else(|| CommandError::failed("the selection is gone"))?;
    let mut bodies: Vec<Body> = Vec::with_capacity(sel.indices.len());
    for &i in &sel.indices {
        let Some(Element::Shape(s)) = siblings.get(i) else {
            return Err(CommandError::failed("Combine Objects works on shapes only"));
        };
        bodies.push(modify::body_of(&s.shape, s.matrix).ok_or_else(|| CommandError::failed("a selected shape is empty"))?);
    }
    let results = modify::combine(op, &bodies);
    // Everything selected is replaced by the results, at the position of the lowest selected object.
    let at = sel.indices.first().copied().unwrap_or(0);
    for &i in sel.indices.iter().rev() {
        siblings.remove(i);
    }
    let count = results.len();
    let mut produced = Vec::new();
    for (n, (_, body)) in results.into_iter().enumerate() {
        let shape = modify::shape_of(&body);
        let center = shape.bounds().map_or(Point::ZERO, |b| b.center());
        let element = Element::Shape(ShapeElement {
            shape,
            is_drawing_object: true,
            matrix: Affine::IDENTITY,
            transformation_point: Some(center),
            ..ShapeElement::default()
        });
        let position = at.saturating_add(n).min(siblings.len());
        siblings.insert(position, element);
        produced.push(position);
    }
    select_paths(doc, &sel, produced);
    Ok(json!({ "op": p.op, "objects": count }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurveParams {
    op: String,
    amount: Option<f64>,
    passes: Option<u32>,
}

fn shape_curves(doc: &mut AniDoc, params: &Value) -> Out {
    let p: CurveParams = parse_params(params)?;
    let op = match p.op.as_str() {
        "smooth" => CurveEdit::Smooth,
        "straighten" => CurveEdit::Straighten,
        "optimize" => CurveEdit::Optimize,
        other => return Err(CommandError::invalid_params(format!("unknown curve operation {other:?}"))),
    };
    let amount = (p.amount.unwrap_or(50.0) / 100.0).clamp(0.0, 1.0);
    let passes = p.passes.unwrap_or(1).clamp(1, 10);
    let (mut before, mut after) = (0_usize, 0_usize);
    let touched = map_shapes(doc, "this command", |shape| {
        let report = modify::edit_curves(shape, op, amount, passes);
        before = before.saturating_add(report.before);
        after = after.saturating_add(report.after);
        report.shape
    })?;
    Ok(json!({ "shapes": touched, "anchors_before": before, "anchors_after": after }))
}

fn lines_to_fills(doc: &mut AniDoc, _params: &Value) -> Out {
    let touched = map_shapes(doc, "Convert Lines to Fills", modify::lines_to_fills)?;
    Ok(json!({ "shapes": touched }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpandParams {
    distance: f64,
    #[serde(default)]
    inset: bool,
}

fn expand_fill(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ExpandParams = parse_params(params)?;
    check_distance(p.distance)?;
    let signed = if p.inset { -p.distance } else { p.distance };
    let touched = map_shapes(doc, "Expand Fill", |s| modify::expand_fills(s, signed))?;
    Ok(json!({ "shapes": touched }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoftenParams {
    distance: f64,
    steps: Option<u32>,
    #[serde(default)]
    inset: bool,
}

fn soften_fill_edges(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SoftenParams = parse_params(params)?;
    check_distance(p.distance)?;
    let signed = if p.inset { -p.distance } else { p.distance };
    let steps = p.steps.unwrap_or(4);
    let touched = map_shapes(doc, "Soften Fill Edges", |s| modify::soften_fills(s, signed, steps))?;
    Ok(json!({ "shapes": touched }))
}

fn check_distance(distance: f64) -> Result<(), CommandError> {
    if distance.is_finite() && distance > 0.0 && distance <= 1000.0 {
        Ok(())
    } else {
        Err(CommandError::invalid_params("distance must be between 0 and 1000 pixels"))
    }
}

#[cfg(test)]
#[allow(
    clippy::many_single_char_names,
    clippy::arithmetic_side_effects,
    clippy::manual_assert_eq,
    clippy::assert_is_empty,
    clippy::redundant_closure_for_method_calls,
    reason = "test code"
)]
mod tests {
    use anicraft_tools::Selection;
    use anim_session::Session;

    use super::*;
    use crate::mcp::registry;

    fn session() -> Session<AniDoc> {
        Session::new(AniDoc::default(), registry())
    }

    fn two_rects(s: &mut Session<AniDoc>) {
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 100, "height": 100, "fill": "#ff0000", "object_drawing": true })).unwrap();
        s.execute("draw.rectangle", &json!({ "x": 50, "y": 50, "width": 100, "height": 100, "fill": "#0000ff", "object_drawing": true })).unwrap();
        s.execute("select.all", &json!({})).unwrap();
    }

    fn elements(s: &Session<AniDoc>) -> usize {
        s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().elements.len()
    }

    #[test]
    fn combine_union_makes_one_drawing_object_and_undoes() {
        let mut s = session();
        two_rects(&mut s);
        assert_eq!(elements(&s), 2);
        s.execute("modify.combine_objects", &json!({ "op": "union" })).unwrap();
        assert_eq!(elements(&s), 1);
        let frame = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().clone();
        let Element::Shape(shape) = &frame.elements[0] else { panic!("not a shape") };
        assert!(shape.is_drawing_object);
        assert!((modify::fill_area(&shape.shape) - 17_500.0).abs() < 5.0);
        assert_eq!(s.doc().editor.state.selection.items.len(), 1);
        s.undo();
        assert_eq!(elements(&s), 2);
    }

    #[test]
    fn combine_punch_crop_intersect() {
        for (op, expected, count) in [("punch", 7_500.0, 1), ("crop", 2_500.0, 1), ("intersect", 2_500.0, 1)] {
            let mut s = session();
            two_rects(&mut s);
            s.execute("modify.combine_objects", &json!({ "op": op })).unwrap();
            assert_eq!(elements(&s), count, "{op}");
            let frame = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().clone();
            let Element::Shape(shape) = &frame.elements[0] else { panic!("not a shape") };
            assert!((modify::fill_area(&shape.shape) - expected).abs() < 5.0, "{op}: {}", modify::fill_area(&shape.shape));
        }
    }

    #[test]
    fn combine_needs_two_shapes() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 10, "height": 10, "object_drawing": true })).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        assert!(s.execute("modify.combine_objects", &json!({ "op": "union" })).is_err());
        assert!(s.execute("modify.combine_objects", &json!({ "op": "melt" })).is_err());
    }

    #[test]
    fn shape_commands_edit_the_selected_shape() {
        let mut s = session();
        s.execute(
            "draw.rectangle",
            &json!({ "x": 0, "y": 0, "width": 100, "height": 100, "fill": "#00aa00", "stroke": { "color": "#000000", "width": 8 }, "object_drawing": true }),
        )
        .unwrap();
        s.execute("select.all", &json!({})).unwrap();
        let area_of = |s: &Session<AniDoc>| {
            let frame = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().clone();
            let Element::Shape(shape) = &frame.elements[0] else { panic!("not a shape") };
            (modify::fill_area(&shape.shape), shape.shape.assemble().strokes.len())
        };
        let (base, strokes) = area_of(&s);
        assert!(strokes > 0);
        s.execute("modify.expand_fill", &json!({ "distance": 10 })).unwrap();
        assert!(area_of(&s).0 > base + 1_000.0);
        s.undo();
        s.execute("modify.convert_lines_to_fills", &json!({})).unwrap();
        assert_eq!(area_of(&s).1, 0, "no strokes remain");
        assert!(area_of(&s).0 > base);
        s.undo();
        s.execute("modify.soften_fill_edges", &json!({ "distance": 10, "steps": 3 })).unwrap();
        assert!(area_of(&s).0 > base);
        s.undo();
        let report = s.execute("modify.shape_curves", &json!({ "op": "optimize", "amount": 80 })).unwrap();
        assert!(report["anchors_after"].as_u64().unwrap() <= report["anchors_before"].as_u64().unwrap());
        s.doc_mut().editor.state.selection = Selection::default();
        assert!(s.execute("modify.expand_fill", &json!({ "distance": 5 })).is_err());
    }
}
