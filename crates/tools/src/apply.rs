//! The reference executor of [`ToolCommand`]s: applies them to a [`Document`] and the [`EditorState`].
//!
//! The application, the command-line tool and the MCP server share this code, so a tool behaves identically
//! wherever it runs. The tools themselves never call it; the host does, once per command, and snapshots the
//! document first when [`ToolCommand::edits_document`] says the command changes it.

use anim_core::{Affine, Point};
use anim_scene::{Document, Element, SceneError, ShapeElement, TextRun, TimelineId, edit};
use thiserror::Error;

use crate::armature::{self, ArmatureError, NewBone};
use crate::command::ToolCommand;
use crate::context::resolve_ref;
use crate::selection::{ElementRef, Selection};
use crate::shapeops;
use crate::state::EditorState;

/// Why a command could not be applied. The document is unchanged when this is returned.
#[derive(Debug, Error)]
pub enum ApplyError {
    #[error(transparent)]
    Scene(#[from] SceneError),
    #[error("no such element: layer {layer}, keyframe {keyframe}, path {path:?}")]
    NoSuchElement { layer: usize, keyframe: u32, path: Vec<usize> },
    #[error("the element is not a {expected}")]
    WrongKind { expected: &'static str },
    #[error("no such symbol: {0}")]
    NoSuchSymbol(String),
    #[error("the transform is not finite")]
    BadMatrix,
    #[error(transparent)]
    Armature(#[from] ArmatureError),
    #[error("the pose is not finite")]
    BadPose,
}

/// What applying a command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Applied {
    /// The document changed.
    pub document_changed: bool,
}

fn no_element(r: &ElementRef) -> ApplyError {
    ApplyError::NoSuchElement { layer: r.layer, keyframe: r.keyframe, path: r.path.clone() }
}

fn container_mut<'a>(doc: &'a mut Document, timeline: &TimelineId, r: &ElementRef, parents: &[usize]) -> Result<&'a mut Vec<Element>, ApplyError> {
    let layer = doc.timeline_mut(timeline).and_then(|t| t.layers.get_mut(r.layer)).ok_or_else(|| no_element(r))?;
    let frame = layer.frames.iter_mut().find(|f| f.index == r.keyframe).ok_or_else(|| no_element(r))?;
    let mut elements = &mut frame.elements;
    for &i in parents {
        let Some(Element::Group(g)) = elements.get_mut(i) else { return Err(no_element(r)) };
        elements = &mut g.members;
    }
    Ok(elements)
}

fn element_mut<'a>(doc: &'a mut Document, timeline: &TimelineId, r: &ElementRef) -> Result<&'a mut Element, ApplyError> {
    let (last, parents) = r.path.split_last().ok_or_else(|| no_element(r))?;
    container_mut(doc, timeline, r, parents)?.get_mut(*last).ok_or_else(|| no_element(r))
}

fn shape_mut<'a>(doc: &'a mut Document, timeline: &TimelineId, r: &ElementRef) -> Result<&'a mut ShapeElement, ApplyError> {
    match element_mut(doc, timeline, r)? {
        Element::Shape(s) => Ok(s),
        _ => Err(ApplyError::WrongKind { expected: "shape" }),
    }
}

/// References sorted so that removing them one by one never invalidates the next: deepest and last first.
fn removal_order(refs: &[ElementRef]) -> Vec<ElementRef> {
    let mut sorted = refs.to_vec();
    sorted.sort_by(|a, b| (b.layer, b.keyframe, &b.path).cmp(&(a.layer, a.keyframe, &a.path)));
    sorted.dedup();
    sorted
}

fn remove_refs(doc: &mut Document, timeline: &TimelineId, refs: &[ElementRef]) {
    for r in removal_order(refs) {
        let Some((last, parents)) = r.path.split_last() else { continue };
        // A selected group's removal already took its selected members with it.
        let Ok(container) = container_mut(doc, timeline, &r, parents) else { continue };
        if *last < container.len() {
            container.remove(*last);
        }
    }
}

fn finite(m: Affine) -> bool {
    m.as_coeffs().iter().all(|c| c.is_finite())
}

fn transform_selection(doc: &mut Document, state: &EditorState, matrix: Affine) -> Result<(), ApplyError> {
    if !finite(matrix) {
        return Err(ApplyError::BadMatrix);
    }
    for r in &state.selection.items {
        let resolved = resolve_ref(doc, &state.timeline, r).ok_or_else(|| no_element(r))?;
        let parent_to_stage = resolved.parent_to_stage;
        let det = parent_to_stage.determinant();
        if !det.is_finite() || det.abs() < 1.0e-12 {
            continue;
        }
        let Some(current) = resolved.element.matrix() else { continue };
        // In stage terms: P * M' = T * P * M.
        let updated = parent_to_stage.inverse() * matrix * parent_to_stage * current;
        if !finite(updated) {
            return Err(ApplyError::BadMatrix);
        }
        element_mut(doc, &state.timeline, r)?.set_matrix(updated);
    }
    Ok(())
}

fn duplicate_selection(doc: &mut Document, state: &mut EditorState) -> Result<(), ApplyError> {
    // Clone each selected element right after itself; process back to front so indices stay valid.
    let mut clones: Vec<(ElementRef, Element)> = Vec::new();
    for r in removal_order(&state.selection.items) {
        let Some(resolved) = resolve_ref(doc, &state.timeline, &r) else { continue };
        clones.push((r, resolved.element.clone()));
    }
    let mut refs: Vec<ElementRef> = Vec::new();
    for (r, element) in &clones {
        let Some((last, parents)) = r.path.split_last() else { continue };
        let container = container_mut(doc, &state.timeline, r, parents)?;
        container.insert(last.saturating_add(1).min(container.len()), element.clone());
    }
    // Final positions: each selected element's clone sits after it, shifted by the selected siblings before it.
    let mut ascending = state.selection.items.clone();
    ascending.sort_by(|a, b| (a.layer, a.keyframe, &a.path).cmp(&(b.layer, b.keyframe, &b.path)));
    ascending.dedup();
    for (i, r) in ascending.iter().enumerate() {
        let before = ascending
            .iter()
            .take(i)
            .filter(|o| {
                o.layer == r.layer
                    && o.keyframe == r.keyframe
                    && o.path.len() == r.path.len()
                    && o.path.split_last().map(|x| x.1) == r.path.split_last().map(|x| x.1)
            })
            .count();
        let mut path = r.path.clone();
        if let Some(last) = path.last_mut() {
            *last = last.saturating_add(1).saturating_add(before);
        }
        refs.push(ElementRef { layer: r.layer, keyframe: r.keyframe, path });
    }
    state.selection.items = refs;
    Ok(())
}

fn layer_mut<'a>(doc: &'a mut Document, timeline: &TimelineId, layer: usize) -> Result<&'a mut anim_scene::Layer, ApplyError> {
    let tl = doc.timeline_mut(timeline).ok_or(SceneError::NoSuchLayer { layer, count: 0 })?;
    let count = tl.layers.len();
    Ok(tl.layers.get_mut(layer).ok_or(SceneError::NoSuchLayer { layer, count })?)
}

fn set_pose(doc: &mut Document, state: &EditorState, layer: usize, frame: u32, pose: &anim_scene::Pose) -> Result<(), ApplyError> {
    if !pose.is_finite() {
        return Err(ApplyError::BadPose);
    }
    let has_armature = layer_mut(doc, &state.timeline, layer)?.armature.is_some();
    if !has_armature {
        return Err(ArmatureError::NoArmature.into());
    }
    if layer_mut(doc, &state.timeline, layer)?.frames.iter().all(|f| f.index != frame) {
        edit::insert_blank_keyframe(doc, &state.timeline, layer, frame)?;
    }
    armature::store_pose(layer_mut(doc, &state.timeline, layer)?, frame, pose)?;
    Ok(())
}

/// Moves an element's transformation point to the stage point `point`; false when the element has none.
fn set_transformation_point(doc: &mut Document, state: &EditorState, target: &ElementRef, point: Point) -> Result<bool, ApplyError> {
    let resolved = resolve_ref(doc, &state.timeline, target).ok_or_else(|| no_element(target))?;
    let to_stage = resolved.local_to_stage();
    let det = to_stage.determinant();
    if !det.is_finite() || det.abs() < 1.0e-12 {
        return Err(ApplyError::BadMatrix);
    }
    let local = to_stage.inverse() * point;
    match element_mut(doc, &state.timeline, target)? {
        Element::Shape(s) => s.transformation_point = Some(local),
        Element::Group(g) => g.transformation_point = Some(local),
        Element::SymbolInstance(i) => i.transformation_point = local,
        _ => return Ok(false),
    }
    Ok(true)
}

/// Applies the armature commands(the other commands never reach here).
fn apply_armature(doc: &mut Document, state: &EditorState, command: &ToolCommand) -> Result<(), ApplyError> {
    match command {
        ToolCommand::AddBone { layer, frame, parent, head, tail, element, kind, name } => {
            let spec = NewBone { parent: *parent, head: *head, tail: *tail, element: *element, name: name.clone(), kind: *kind };
            armature::add_bone(layer_mut(doc, &state.timeline, *layer)?, *frame, &spec)?;
        }
        ToolCommand::SetPose { layer, frame, pose } => set_pose(doc, state, *layer, *frame, pose)?,
        ToolCommand::SetBoneLimits { layer, bone, limits } => {
            if limits.is_some_and(|(lo, hi)| !lo.is_finite() || !hi.is_finite()) {
                return Err(ApplyError::BadPose);
            }
            let slot = armature_of(doc, &state.timeline, *layer)?.bones.get_mut(*bone).ok_or(ArmatureError::NoSuchBone(*bone))?;
            slot.limits = limits.map(|(lo, hi)| if lo <= hi { (lo, hi) } else { (hi, lo) });
        }
        ToolCommand::RenameBone { layer, bone, name } => {
            armature_of(doc, &state.timeline, *layer)?.bones.get_mut(*bone).ok_or(ArmatureError::NoSuchBone(*bone))?.name.clone_from(name);
        }
        ToolCommand::RemoveBone { layer, bone } => armature::remove_bone(layer_mut(doc, &state.timeline, *layer)?, *bone)?,
        ToolCommand::RemoveArmature { layer } => {
            let l = layer_mut(doc, &state.timeline, *layer)?;
            if l.armature.is_none() {
                return Err(ArmatureError::NoArmature.into());
            }
            armature::remove_armature(l);
        }
        _ => {}
    }
    Ok(())
}

fn armature_of<'a>(doc: &'a mut Document, timeline: &TimelineId, layer: usize) -> Result<&'a mut anim_scene::Armature, ApplyError> {
    layer_mut(doc, timeline, layer)?.armature.as_mut().ok_or_else(|| ArmatureError::NoArmature.into())
}

fn center_of(shape: &anim_shape::EdgeShape) -> Option<Point> {
    shape.bounds().map(|b| b.center())
}

fn draw_shape(
    doc: &mut Document,
    state: &mut EditorState,
    layer: usize,
    frame: u32,
    shape: &anim_shape::EdgeShape,
    object_drawing: bool,
) -> Result<(), ApplyError> {
    let timeline = state.timeline.clone();
    let layer_count = doc.timeline(&timeline).map_or(0, |t| t.layers.len());
    if layer >= layer_count {
        return Err(SceneError::NoSuchLayer { layer, count: layer_count }.into());
    }
    if object_drawing {
        let mut element = ShapeElement::new(shape.clone());
        element.is_drawing_object = true;
        element.transformation_point = center_of(shape);
        let index = edit::add_element(doc, &timeline, layer, frame, Element::Shape(element))?;
        let keyframe = doc.timeline(&timeline).and_then(|t| t.keyframe_at(layer, frame)).map_or(frame, |f| f.index);
        state.selection = Selection { items: vec![ElementRef::top_level(layer, keyframe, index)], scope: None };
        return Ok(());
    }
    if doc.timeline(&timeline).and_then(|t| t.keyframe_at(layer, frame)).is_none() {
        edit::insert_blank_keyframe(doc, &timeline, layer, frame)?;
    }
    let keyframe = edit::keyframe_mut(doc, &timeline, layer, frame)?;
    let existing = keyframe.elements.iter_mut().find_map(|e| match e {
        Element::Shape(s) if !s.is_drawing_object && s.matrix == Affine::IDENTITY => Some(s),
        _ => None,
    });
    match existing {
        Some(s) => s.shape = shapeops::merge(&s.shape, shape),
        None => keyframe.elements.push(Element::Shape(ShapeElement::new(shape.clone()))),
    }
    Ok(())
}

/// Splits `offset` over the runs of a text field: the run it falls into and the offset inside that run.
fn run_position(runs: &[TextRun], offset: usize) -> Option<(usize, usize)> {
    let mut start = 0_usize;
    for (i, run) in runs.iter().enumerate() {
        let end = start.saturating_add(run.characters.len());
        if offset <= end {
            return Some((i, offset.saturating_sub(start)));
        }
        start = end;
    }
    runs.len().checked_sub(1).map(|i| (i, runs.get(i).map_or(0, |r| r.characters.len())))
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i = i.saturating_sub(1);
    }
    i
}

fn insert_text(doc: &mut Document, state: &EditorState, target: &ElementRef, offset: usize, text: &str) -> Result<(), ApplyError> {
    let Element::Text(field) = element_mut(doc, &state.timeline, target)? else { return Err(ApplyError::WrongKind { expected: "text field" }) };
    if field.text_runs.is_empty() {
        let s = &state.settings.text;
        field.text_runs.push(TextRun::new("", s.face.clone(), s.size));
    }
    let Some((run_index, inner)) = run_position(&field.text_runs, offset) else { return Ok(()) };
    if let Some(run) = field.text_runs.get_mut(run_index) {
        let at = floor_boundary(&run.characters, inner);
        run.characters.insert_str(at, text);
    }
    Ok(())
}

fn delete_text(doc: &mut Document, state: &EditorState, target: &ElementRef, start: usize, end: usize) -> Result<(), ApplyError> {
    let Element::Text(field) = element_mut(doc, &state.timeline, target)? else { return Err(ApplyError::WrongKind { expected: "text field" }) };
    let mut run_start = 0_usize;
    for run in &mut field.text_runs {
        let run_end = run_start.saturating_add(run.characters.len());
        let from = floor_boundary(&run.characters, start.max(run_start).saturating_sub(run_start));
        let to = floor_boundary(&run.characters, end.min(run_end).saturating_sub(run_start));
        if end > run_start && start < run_end && from < to {
            run.characters.replace_range(from..to, "");
        }
        run_start = run_end;
    }
    // A run that lost everything stays (it carries the style); empty runs beyond the first are dropped.
    let mut first = true;
    field.text_runs.retain(|r| !r.characters.is_empty() || std::mem::take(&mut first));
    Ok(())
}

/// Applies one command.
///
/// With [`EditorState::auto_keyframe`] on, a command that changes the artwork of a held frame first makes that
/// frame a keyframe (see [`crate::autokey`]).
///
/// # Errors
/// [`ApplyError`] when a reference does not resolve, an element has the wrong kind or a document edit is refused.
pub fn apply(doc: &mut Document, state: &mut EditorState, command: &ToolCommand) -> Result<Applied, ApplyError> {
    if state.auto_keyframe
        && command.edits_document()
        && let Some(rewritten) = crate::autokey::prepare(doc, state, command)?
    {
        return apply_one(doc, state, &rewritten);
    }
    apply_one(doc, state, command)
}

fn apply_one(doc: &mut Document, state: &mut EditorState, command: &ToolCommand) -> Result<Applied, ApplyError> {
    let mut changed = command.edits_document();
    match command {
        ToolCommand::SetSelection { selection } => state.selection = selection.clone(),
        ToolCommand::DeleteSelection => {
            let items = state.selection.items.clone();
            remove_refs(doc, &state.timeline, &items);
            state.selection.items.clear();
        }
        ToolCommand::DuplicateSelection => duplicate_selection(doc, state)?,
        ToolCommand::TransformSelection { matrix } => transform_selection(doc, state, *matrix)?,
        ToolCommand::SetTransformationPoint { target, point } => changed = set_transformation_point(doc, state, target, *point)?,
        ToolCommand::DrawShape { layer, frame, shape, object_drawing } => draw_shape(doc, state, *layer, *frame, shape, *object_drawing)?,
        ToolCommand::AddElement { layer, frame, element } => {
            let timeline = state.timeline.clone();
            let index = edit::add_element(doc, &timeline, *layer, *frame, element.clone())?;
            let keyframe = doc.timeline(&timeline).and_then(|t| t.keyframe_at(*layer, *frame)).map_or(*frame, |f| f.index);
            state.selection = Selection { items: vec![ElementRef::top_level(*layer, keyframe, index)], scope: None };
        }
        ToolCommand::ReplaceShape { target, shape } => shape_mut(doc, &state.timeline, target)?.shape = shape.clone(),
        ToolCommand::DeleteElements { targets } => {
            remove_refs(doc, &state.timeline, targets);
            state.selection.items.retain(|s| !targets.contains(s));
        }
        ToolCommand::InsertText { target, offset, text } => insert_text(doc, state, target, *offset, text)?,
        ToolCommand::DeleteText { target, start, end } => delete_text(doc, state, target, *start, *end)?,
        ToolCommand::CaretMotion { .. } => {}
        ToolCommand::BeginTextEdit { target } => state.text_editing = Some(target.clone()),
        ToolCommand::EndTextEdit => state.text_editing = None,
        ToolCommand::EnterGroup { target } => {
            match resolve_ref(doc, &state.timeline, target).map(|r| r.element) {
                Some(Element::Group(_)) => {}
                Some(_) => return Err(ApplyError::WrongKind { expected: "group" }),
                None => return Err(no_element(target)),
            }
            state.selection = Selection { items: Vec::new(), scope: Some(target.clone()) };
        }
        ToolCommand::EnterSymbol { target } => {
            let (name, to_stage) = match resolve_ref(doc, &state.timeline, target) {
                Some(r) => match r.element {
                    Element::SymbolInstance(i) => (i.library_item_name.clone(), r.local_to_stage()),
                    _ => return Err(ApplyError::WrongKind { expected: "symbol instance" }),
                },
                None => return Err(no_element(target)),
            };
            if doc.library.find_symbol(&name).is_none() {
                return Err(ApplyError::NoSuchSymbol(name));
            }
            // Double-clicking an instance edits its symbol in place, in the context of where it sits.
            state.enter_symbol(&name, Some(to_stage));
        }
        ToolCommand::ExitScope => {
            if let Some(scope) = state.selection.scope.take() {
                state.selection.items = vec![scope];
            } else {
                state.leave_symbol();
            }
        }
        ToolCommand::SetToolStyle { fill, stroke } => {
            if let Some(f) = fill {
                state.settings.fill = f.clone();
                state.settings.fill_enabled = true;
            }
            if let Some(s) = stroke {
                state.settings.stroke = s.clone();
                state.settings.stroke_enabled = true;
            }
        }
        ToolCommand::AddBone { .. }
        | ToolCommand::SetPose { .. }
        | ToolCommand::SetBoneLimits { .. }
        | ToolCommand::RenameBone { .. }
        | ToolCommand::RemoveBone { .. }
        | ToolCommand::RemoveArmature { .. } => apply_armature(doc, state, command)?,
        ToolCommand::ActivateTool { tool } => state.active_tool = *tool,
        ToolCommand::PanScreen { delta } => state.view.offset += *delta,
        ToolCommand::ZoomAt { center, factor } => state.view.zoom_at(*center, *factor),
        ToolCommand::ZoomToRect { rect } => state.view.zoom_to_rect(*rect),
    }
    // Indices into a layer may have been invalidated; drop selection entries that no longer resolve.
    state.selection.items.retain(|r| resolve_ref(doc, &state.timeline, r).is_some());
    if state.selection.scope.as_ref().is_some_and(|s| resolve_ref(doc, &state.timeline, s).is_none()) {
        state.selection.scope = None;
    }
    Ok(Applied { document_changed: changed })
}
