//! Auto keyframe: changing artwork on a held frame makes that frame a keyframe first.
//!
//! A frame inside a span shows the keyframe the span starts at. Drawing on it, or moving something that is on it,
//! would change the whole span. With auto keyframe on, Animate instead turns the playhead frame into a keyframe
//! (a copy of what was showing) and changes that one. [`prepare`] does the conversion before a command runs and
//! points the command, and the selection, at the new keyframe.

use anim_scene::{Document, TimelineId, edit};

use crate::apply::ApplyError;
use crate::command::ToolCommand;
use crate::selection::ElementRef;
use crate::state::EditorState;

/// True when `layer` shows the keyframe `keyframe` at `frame` without that keyframe starting there.
fn held(doc: &Document, timeline: &TimelineId, layer: usize, keyframe: u32, frame: u32) -> bool {
    doc.timeline(timeline).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).is_some_and(|k| k.index == keyframe && keyframe != frame)
}

/// The element references a command reads, as the selection or its own targets.
fn refs_of<'a>(state: &'a EditorState, command: &'a ToolCommand) -> Vec<&'a ElementRef> {
    match command {
        ToolCommand::DeleteSelection | ToolCommand::DuplicateSelection | ToolCommand::TransformSelection { .. } | ToolCommand::Transform3DSelection { .. } => {
            state.selection.items.iter().collect()
        }
        ToolCommand::SetTransformationPoint { target, .. }
        | ToolCommand::SetCenterPoint3D { target, .. }
        | ToolCommand::ReplaceShape { target, .. }
        | ToolCommand::SetPrimitive { target, .. }
        | ToolCommand::InsertText { target, .. }
        | ToolCommand::DeleteText { target, .. } => vec![target],
        ToolCommand::DeleteElements { targets } => targets.iter().collect(),
        _ => Vec::new(),
    }
}

/// Makes the held frames `command` would change into keyframes.
///
/// Returns the command rewritten to point at the new keyframes when it names elements (the selection is rewritten in
/// place), and `None` when the command can run as it is: nothing was held, or it addresses a frame by number.
///
/// # Errors
/// The refusal of the keyframe insertion (the frame limit).
pub fn prepare(doc: &mut Document, state: &mut EditorState, command: &ToolCommand) -> Result<Option<ToolCommand>, ApplyError> {
    let timeline = state.timeline.clone();
    let frame = state.frame;
    if let ToolCommand::DrawShape { layer, frame: at, .. } | ToolCommand::AddElement { layer, frame: at, .. } = command {
        let covering = doc.timeline(&timeline).and_then(|t| t.layers.get(*layer)).and_then(|l| l.keyframe_at(*at)).map(|k| k.index);
        if covering.is_some_and(|index| index != *at) {
            edit::insert_keyframe(doc, &timeline, *layer, *at)?;
        }
        return Ok(None);
    }
    let mut converted: Vec<(usize, u32)> = Vec::new();
    for r in refs_of(state, command) {
        if held(doc, &timeline, r.layer, r.keyframe, frame) && !converted.contains(&(r.layer, r.keyframe)) {
            converted.push((r.layer, r.keyframe));
        }
    }
    if converted.is_empty() {
        return Ok(None);
    }
    for (layer, _) in &converted {
        edit::insert_keyframe(doc, &timeline, *layer, frame)?;
    }
    let point_at_new = |r: &mut ElementRef| {
        if converted.contains(&(r.layer, r.keyframe)) {
            r.keyframe = frame;
        }
    };
    state.selection.items.iter_mut().for_each(point_at_new);
    if let Some(scope) = state.selection.scope.as_mut() {
        point_at_new(scope);
    }
    if let Some(editing) = state.text_editing.as_mut() {
        point_at_new(editing);
    }
    let mut rewritten = command.clone();
    match &mut rewritten {
        ToolCommand::SetTransformationPoint { target, .. }
        | ToolCommand::SetCenterPoint3D { target, .. }
        | ToolCommand::ReplaceShape { target, .. }
        | ToolCommand::SetPrimitive { target, .. }
        | ToolCommand::InsertText { target, .. }
        | ToolCommand::DeleteText { target, .. } => point_at_new(target),
        ToolCommand::DeleteElements { targets } => targets.iter_mut().for_each(point_at_new),
        _ => {}
    }
    Ok(Some(rewritten))
}

#[cfg(test)]
mod tests {
    use anim_core::kurbo::Rect as KRect;
    use anim_core::kurbo::Shape as _;
    use anim_core::{Affine, Rgba8};
    use anim_scene::{Element, ShapeElement};
    use anim_shape::{EdgeShape, FillStyle, PathInput};

    use super::*;
    use crate::selection::Selection;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> EdgeShape {
        EdgeShape::from_paths(&[PathInput::new(KRect::new(x0, y0, x1, y1).to_path(0.1), Some(FillStyle::Solid(Rgba8::BLACK)), None)])
    }

    fn square() -> Element {
        Element::Shape(ShapeElement::new(rect(0.0, 0.0, 10.0, 10.0)))
    }

    /// A document whose layer 0 holds one drawing object on a keyframe at 0 lasting 10 frames.
    fn held_doc() -> (Document, EditorState) {
        let mut doc = Document::default();
        let tl = TimelineId::Scene(0);
        edit::insert_frames(&mut doc, &tl, 0, 1, 9).unwrap();
        edit::add_element(&mut doc, &tl, 0, 0, square()).unwrap();
        let mut state = EditorState::new();
        state.auto_keyframe = true;
        (doc, state)
    }

    fn keyframes(doc: &Document) -> Vec<(u32, u32, usize)> {
        doc.scenes[0].layers[0].frames.iter().map(|f| (f.index, f.duration, f.elements.len())).collect()
    }

    #[test]
    fn moving_on_a_held_frame_splits_the_span_and_moves_only_the_new_keyframe() {
        let (mut doc, mut state) = held_doc();
        state.frame = 4;
        state.selection = Selection { items: vec![ElementRef::top_level(0, 0, 0)], scope: None };
        crate::apply(&mut doc, &mut state, &ToolCommand::TransformSelection { matrix: Affine::translate((5.0, 0.0)) }).unwrap();
        assert_eq!(keyframes(&doc), vec![(0, 4, 1), (4, 6, 1)]);
        assert_eq!(state.selection.items, vec![ElementRef::top_level(0, 4, 0)]);
        let frames = &doc.scenes[0].layers[0].frames;
        let moved = frames[1].elements[0].matrix().unwrap();
        let kept = frames[0].elements[0].matrix().unwrap();
        assert!((moved.as_coeffs()[4] - 5.0).abs() < 1e-9, "the new keyframe moved");
        assert!(kept.as_coeffs()[4].abs() < 1e-9, "the original keyframe stayed");
    }

    #[test]
    fn drawing_on_a_held_frame_makes_a_keyframe_with_the_old_content_plus_the_drawing() {
        let (mut doc, mut state) = held_doc();
        state.frame = 3;
        let shape = rect(20.0, 20.0, 30.0, 30.0);
        crate::apply(&mut doc, &mut state, &ToolCommand::DrawShape { layer: 0, frame: 3, shape, object_drawing: true }).unwrap();
        assert_eq!(keyframes(&doc), vec![(0, 3, 1), (3, 7, 2)]);
    }

    #[test]
    fn it_is_off_by_default_and_a_keyframe_start_is_left_alone() {
        let (mut doc, mut state) = held_doc();
        state.auto_keyframe = false;
        state.frame = 4;
        state.selection = Selection { items: vec![ElementRef::top_level(0, 0, 0)], scope: None };
        crate::apply(&mut doc, &mut state, &ToolCommand::TransformSelection { matrix: Affine::translate((1.0, 0.0)) }).unwrap();
        assert_eq!(keyframes(&doc), vec![(0, 10, 1)], "without auto keyframe the span is edited as a whole");
        state.auto_keyframe = true;
        state.frame = 0;
        crate::apply(&mut doc, &mut state, &ToolCommand::TransformSelection { matrix: Affine::translate((1.0, 0.0)) }).unwrap();
        assert_eq!(keyframes(&doc), vec![(0, 10, 1)], "the frame is already a keyframe");
    }
}
