//! Arranging the selection: Group, Ungroup, Break Apart, Distribute to Layers and Keyframes, Lock and Unlock All, the
//! Modify > Transform submenu and Paste.
//!
//! The selection may sit inside groups: the objects of one container (a keyframe or a group) are grouped, ungrouped
//! and broken apart together, so the commands work while a group is being edited. Distribution works on the objects
//! of one keyframe.

use anicraft_tools::lock::set_locked;
use anicraft_tools::{ElementRef, Selection, ToolCommand};
use anim_core::{Affine, Point, Rect};
use anim_scene::timeline::LayerKind;
use anim_scene::{BitmapItem, Document, Element, Group, Library, LibraryItem, ShapeElement, TimelineId, edit};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use anim_shape::{BitmapFill, EdgeShape, FillStyle, PathInput};
use serde::Deserialize;
use serde_json::{Value, json};

use super::doc::AniDoc;

pub use super::editing::element_mut;

type Out = Result<Value, CommandError>;

const NO_PARAMS: &str = r#"{"type":"object","additionalProperties":false,"properties":{}}"#;

/// Group nesting that Break Apart and Ungroup will take apart in one step.
const MAX_MEMBERS: usize = 100_000;

pub(super) fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::failed(e.to_string())
}

pub(super) const fn has_selection(doc: &AniDoc) -> bool {
    !doc.editor.state.selection.is_empty()
}

/// The selected objects of one container, in stacking order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sel {
    pub layer: usize,
    pub keyframe: u32,
    /// Path of the group holding the objects; empty for the keyframe itself.
    pub parent: Vec<usize>,
    /// Indices in the container, ascending.
    pub indices: Vec<usize>,
}

impl Sel {
    fn reference(&self, index: usize) -> ElementRef {
        let mut path = self.parent.clone();
        path.push(index);
        ElementRef { layer: self.layer, keyframe: self.keyframe, path }
    }
}

/// The selection, when all of it lies in one container.
pub(super) fn one_parent(doc: &AniDoc) -> Result<Sel, CommandError> {
    let items = &doc.editor.state.selection.items;
    let first = items.first().ok_or_else(|| CommandError::failed("nothing is selected"))?;
    let (index, parent) = first.path.split_last().ok_or_else(|| CommandError::failed("the selection is broken"))?;
    let mut indices = vec![*index];
    for r in items.iter().skip(1) {
        let Some((i, p)) = r.path.split_last() else { return Err(CommandError::failed("the selection is broken")) };
        if r.layer != first.layer || r.keyframe != first.keyframe || p != parent {
            return Err(CommandError::failed("the selection spans several layers, keyframes or groups; select objects of one container"));
        }
        indices.push(*i);
    }
    indices.sort_unstable();
    indices.dedup();
    Ok(Sel { layer: first.layer, keyframe: first.keyframe, parent: parent.to_vec(), indices })
}

/// The element list a [`Sel`] points into.
pub(super) fn siblings_mut<'a>(document: &'a mut Document, id: &TimelineId, sel: &Sel) -> Option<&'a mut Vec<Element>> {
    let layer = document.timeline_mut(id)?.layers.get_mut(sel.layer)?;
    let frame = layer.frames.iter_mut().find(|f| f.index == sel.keyframe)?;
    let mut elements = &mut frame.elements;
    for &i in &sel.parent {
        let Some(Element::Group(g)) = elements.get_mut(i) else { return None };
        elements = &mut g.members;
    }
    Some(elements)
}

/// Selects the objects at `indices` of the container `sel` points into.
pub(super) fn select_paths(doc: &mut AniDoc, sel: &Sel, indices: impl IntoIterator<Item = usize>) {
    let items = indices.into_iter().map(|i| sel.reference(i)).collect();
    doc.editor.state.selection = doc.editor.state.selection.with_items(items);
}

/// Registers the commands of this module.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(all()).expect("arrange command ids are unique");
}

/// The commands.
#[must_use]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("modify.group", "Group", group)
            .in_menu(&["Modify"])
            .with_shortcut("Cmd+G")
            .enabled_when(has_selection)
            .describe("Groups the selected objects (of one keyframe, or of the group being edited).")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.ungroup", "Ungroup", ungroup)
            .in_menu(&["Modify"])
            .with_shortcut("Cmd+Shift+G")
            .enabled_when(has_selection)
            .describe("Replaces each selected group by its members, with the group's transformation applied to them.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.break_apart", "Break Apart", break_apart)
            .in_menu(&["Modify"])
            .with_shortcut("Cmd+B")
            .enabled_when(has_selection)
            .describe("Breaks the selected objects apart: groups into their members, symbol instances into the content of their first frame, text fields of several left-to-right letters into one field per letter (the next press turns each letter, like any single letter or right-to-left text, into shapes: the glyph outlines), bitmaps into shapes filled with the bitmap and drawing objects into merge-mode shapes that merge with the others of the keyframe.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.distribute_to_layers", "Distribute to Layers", distribute_to_layers)
            .in_menu(&["Modify", "Timeline"])
            .enabled_when(has_selection)
            .describe("Puts every selected object of the keyframe on a layer of its own above the current layer, named after the symbol, bitmap or text, with the same keyframe structure. The topmost object gets the top layer.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.distribute_to_keyframes", "Distribute to Keyframes", distribute_to_keyframes)
            .in_menu(&["Modify", "Timeline"])
            .enabled_when(has_selection)
            .describe("Puts the selected objects of the keyframe on consecutive keyframes of the same layer, the bottom-most object staying on this keyframe, the next one on the next frame, and so on.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.lock", "Lock", lock_selection)
            .in_menu(&["Modify", "Arrange"])
            .with_shortcut("Cmd+Alt+L")
            .enabled_when(has_selection)
            .describe("Locks the selected objects: they stay on the stage but cannot be picked or edited until Unlock All.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.unlock_all", "Unlock All", unlock_all)
            .in_menu(&["Modify", "Arrange"])
            .with_shortcut("Cmd+Alt+Shift+L")
            .describe("Unlocks every locked object of the timeline being edited.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.transform", "Transform", transform)
            .in_menu(&["Modify", "Transform"])
            .enabled_when(has_selection)
            .describe("Transforms the selection about the centre of its bounds. `op`: rotate_90_cw, rotate_90_ccw, scale_rotate (`scale` in percent, default 100, and `degrees` clockwise, default 0) or remove (drops scale, rotation and skew of each object, keeping its position).")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["op"],"properties":{"op":{"enum":["rotate_90_cw","rotate_90_ccw","scale_rotate","remove"]},"scale":{"type":"number","exclusiveMinimum":0,"maximum":100000},"degrees":{"type":"number"}}}"#),
        CommandSpec::new("edit.paste", "Paste", paste)
            .in_menu(&["Edit"])
            .with_shortcut("Cmd+V")
            .describe("Pastes `elements` (as `edit_copy` returned them) onto the current layer and frame. `position`: offset (default, moved by `offset`, 12 px each way unless given), in_place (at the original coordinates) or center (centred on the stage). With `as_bitmap` the objects are pasted as one bitmap of their rendering.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["elements"],"properties":{"elements":{"type":"array","items":{}},"position":{"enum":["offset","in_place","center"]},"offset":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},"as_bitmap":{"type":"boolean","default":false}}}"#),
    ]
}

// ---------------------------------------------------------------------------------------------------------------------
// Group, ungroup, break apart
// ---------------------------------------------------------------------------------------------------------------------

fn group(doc: &mut AniDoc, _params: &Value) -> Out {
    let sel = one_parent(doc)?;
    let id = doc.editor.state.timeline.clone();
    let siblings = siblings_mut(doc.document_mut(), &id, &sel).ok_or_else(|| CommandError::failed("the selection is gone"))?;
    if sel.indices.iter().any(|&i| i >= siblings.len()) {
        return Err(CommandError::failed("the selection is stale"));
    }
    let mut members = Vec::with_capacity(sel.indices.len());
    for &i in sel.indices.iter().rev() {
        members.push(siblings.remove(i));
    }
    members.reverse();
    let count = members.len();
    let at = sel.indices.first().copied().unwrap_or(0).min(siblings.len());
    siblings.insert(at, Element::Group(Group::new(members)));
    select_paths(doc, &sel, [at]);
    Ok(json!({ "grouped": count, "index": at }))
}

fn ungroup(doc: &mut AniDoc, _params: &Value) -> Out {
    expand(doc, false)
}

fn break_apart(doc: &mut AniDoc, _params: &Value) -> Out {
    expand(doc, true)
}

fn compose(outer: Affine, member: &mut Element) {
    if let Some(m) = member.matrix() {
        member.set_matrix(outer * m);
    }
}

/// The content a symbol instance shows at its first frame, bottom layer first, placed in the instance's space.
fn instance_content(library: &Library, instance: &anim_scene::SymbolInstance) -> Vec<Element> {
    let Some(symbol) = library.find_symbol(&instance.library_item_name) else { return Vec::new() };
    let mut out = Vec::new();
    for layer in symbol.timeline.layers.iter().rev().filter(|l| l.kind == LayerKind::Normal) {
        if let Some(frame) = layer.keyframe_at(instance.first_frame) {
            out.extend(frame.elements.iter().cloned());
        }
    }
    for e in &mut out {
        compose(instance.matrix, e);
    }
    out
}

/// A bitmap instance as a shape filled with the bitmap.
fn bitmap_shape(library: &Library, instance: &anim_scene::BitmapInstance) -> Option<ShapeElement> {
    let item: &BitmapItem = library.items.iter().find_map(|i| match i {
        LibraryItem::Bitmap(b) if b.name == instance.library_item_name => Some(b),
        _ => None,
    })?;
    let [w, h] = item.size?;
    let mut path = anim_core::BezPath::new();
    path.move_to((0.0, 0.0));
    path.line_to((w, 0.0));
    path.line_to((w, h));
    path.line_to((0.0, h));
    path.close_path();
    let fill = FillStyle::Bitmap(BitmapFill { path: item.name.clone(), matrix: Affine::scale(20.0), clipped: true, smoothing: item.smoothing });
    let shape = EdgeShape::from_paths(&[PathInput::new(path, Some(fill), None)]);
    Some(ShapeElement { shape, matrix: instance.matrix, extra: instance.extra.clone(), ..ShapeElement::default() })
}

/// The glyph outlines of a text field as a merge-mode shape.
fn text_shape(engine: &mut anim_text::TextEngine, text: &anim_scene::TextElement) -> Option<ShapeElement> {
    let layout = super::letters::laid_out(engine, text)?;
    let nodes = engine.to_display(&layout);
    let mut inputs = Vec::new();
    collect_fills(&nodes, Affine::translate((super::letters::left_of(text), 0.0)), &mut inputs, 0);
    if inputs.is_empty() {
        return None;
    }
    Some(ShapeElement { shape: EdgeShape::from_paths(&inputs), matrix: text.matrix, ..ShapeElement::default() })
}

fn collect_fills(nodes: &[anim_display::Node], to_parent: Affine, out: &mut Vec<PathInput>, depth: usize) {
    if depth > 8 {
        return;
    }
    for node in nodes {
        match node {
            anim_display::Node::Fill(f) => {
                if let anim_display::Paint::Solid(color) = f.paint {
                    out.push(PathInput::new(to_parent * f.transform * f.path.clone(), Some(FillStyle::Solid(color)), None));
                }
            }
            anim_display::Node::Group(g) => collect_fills(&g.children, to_parent * g.transform, out, depth.saturating_add(1)),
            _ => {}
        }
    }
}

/// The merge-mode shape of a keyframe (identity matrix) the broken-apart shapes join, if there is one.
fn merge_target(elements: &[Element]) -> Option<usize> {
    elements.iter().position(|e| matches!(e, Element::Shape(s) if !s.is_drawing_object && s.matrix == Affine::IDENTITY))
}

/// Replaces groups (and, with `everything`, every other breakable object) by their content.
fn expand(doc: &mut AniDoc, everything: bool) -> Out {
    let sel = one_parent(doc)?;
    let id = doc.editor.state.timeline.clone();
    let library = doc.document().library.clone();
    let mut engine: Option<anim_text::TextEngine> = None;
    let siblings = siblings_mut(doc.document_mut(), &id, &sel).ok_or_else(|| CommandError::failed("the selection is gone"))?;
    let mut result: Vec<Element> = Vec::with_capacity(siblings.len());
    let mut produced = Vec::new();
    let mut loose_shapes: Vec<ShapeElement> = Vec::new();
    let mut touched = 0_usize;
    for (i, element) in std::mem::take(siblings).into_iter().enumerate() {
        if !sel.indices.contains(&i) {
            result.push(element);
            continue;
        }
        let mut members: Vec<Element> = match element {
            Element::Group(g) => {
                touched = touched.saturating_add(1);
                let mut members = g.members;
                for m in &mut members {
                    compose(g.matrix, m);
                }
                members
            }
            Element::SymbolInstance(inst) if everything => {
                touched = touched.saturating_add(1);
                instance_content(&library, &inst)
            }
            Element::Text(text) if everything => {
                let engine = engine.get_or_insert_with(anim_text::TextEngine::new);
                // A field of several letters first becomes one field per letter; a letter becomes a shape.
                if let Some(letters) = super::letters::split(engine, &text, MAX_MEMBERS) {
                    touched = touched.saturating_add(1);
                    letters.into_iter().map(|t| Element::Text(Box::new(t))).collect()
                } else {
                    text_shape(engine, &text).map_or_else(
                        || vec![Element::Text(text)],
                        |shape| {
                            touched = touched.saturating_add(1);
                            loose_shapes.push(shape);
                            Vec::new()
                        },
                    )
                }
            }
            Element::Bitmap(bitmap) if everything => bitmap_shape(&library, &bitmap).map_or_else(
                || vec![Element::Bitmap(bitmap)],
                |shape| {
                    touched = touched.saturating_add(1);
                    vec![Element::Shape(shape)]
                },
            ),
            Element::Shape(mut shape) if everything && shape.is_drawing_object => {
                touched = touched.saturating_add(1);
                shape.shape.transform(shape.matrix);
                shape.matrix = Affine::IDENTITY;
                shape.is_drawing_object = false;
                shape.transformation_point = None;
                shape.primitive = None;
                loose_shapes.push(shape);
                Vec::new()
            }
            other => vec![other],
        };
        if members.len() > MAX_MEMBERS {
            members.truncate(MAX_MEMBERS);
        }
        for m in members {
            produced.push(result.len());
            result.push(m);
        }
    }
    // Shapes that stopped being objects join the keyframe's merge-mode shape.
    for loose in loose_shapes {
        let mut baked = loose;
        baked.shape.transform(baked.matrix);
        baked.matrix = Affine::IDENTITY;
        if let Some(at) = merge_target(&result) {
            if let Some(Element::Shape(target)) = result.get_mut(at) {
                target.shape = anicraft_tools::shapeops::merge(&target.shape, &baked.shape);
            }
            produced.push(at);
        } else {
            produced.push(result.len());
            result.push(Element::Shape(baked));
        }
    }
    *siblings_mut(doc.document_mut(), &id, &sel).ok_or_else(|| CommandError::failed("the selection is gone"))? = result;
    if touched == 0 {
        return Err(CommandError::failed(if everything { "nothing selected can be broken apart" } else { "no group is selected" }));
    }
    produced.sort_unstable();
    produced.dedup();
    select_paths(doc, &sel, produced);
    Ok(json!({ "expanded": touched }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Distribute
// ---------------------------------------------------------------------------------------------------------------------

fn leaf(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn layer_name_of(element: &Element) -> Option<String> {
    match element {
        Element::SymbolInstance(s) => Some(leaf(&s.library_item_name).to_owned()),
        Element::Bitmap(b) => Some(leaf(&b.library_item_name).to_owned()),
        Element::Text(t) => {
            let text: String = t.plain_text().chars().filter(|c| !c.is_control()).take(24).collect();
            let text = text.trim().to_owned();
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}

fn top_level(doc: &AniDoc) -> Result<Sel, CommandError> {
    let sel = one_parent(doc)?;
    if sel.parent.is_empty() { Ok(sel) } else { Err(CommandError::failed("select objects of the keyframe itself, not members of a group")) }
}

fn distribute_to_layers(doc: &mut AniDoc, _params: &Value) -> Out {
    let sel = top_level(doc)?;
    let id = doc.editor.state.timeline.clone();
    let (frames, parent): (Vec<(u32, u32)>, Option<usize>) = {
        let layer = doc.document().timeline(&id).and_then(|t| t.layers.get(sel.layer)).ok_or_else(|| CommandError::failed("no such layer"))?;
        (layer.frames.iter().map(|f| (f.index, f.duration)).collect(), layer.parent_layer_index)
    };
    let names: Vec<Option<String>> = {
        let siblings = siblings_mut(doc.document_mut(), &id, &sel).ok_or_else(|| CommandError::failed("the selection is gone"))?;
        sel.indices.iter().map(|&i| siblings.get(i).and_then(layer_name_of)).collect()
    };
    let elements = edit::remove_elements(doc.document_mut(), &id, sel.layer, sel.keyframe, &sel.indices).map_err(failed)?;
    let count = elements.len();
    for (element, name) in elements.into_iter().zip(names) {
        let at = edit::add_layer(doc.document_mut(), &id, sel.layer, name.as_deref(), LayerKind::Normal).map_err(failed)?;
        let tl = doc.document_mut().timeline_mut(&id).ok_or_else(|| CommandError::failed("the timeline is gone"))?;
        if let Some(layer) = tl.layers.get_mut(at) {
            layer.frames = frames.iter().map(|&(index, duration)| anim_scene::Frame::blank(index, duration)).collect();
            layer.parent_layer_index = parent;
            if let Some(frame) = layer.frames.iter_mut().find(|f| f.index == sel.keyframe) {
                frame.elements.push(element);
            }
        }
    }
    // The first object went to the lowest of the new layers; the source layer moved down by `count`.
    let items = (0..count).map(|k| ElementRef::top_level(sel.layer.saturating_add(count.saturating_sub(1).saturating_sub(k)), sel.keyframe, 0)).collect();
    doc.editor.state.selection = Selection { items, scope: None };
    doc.editor.state.layer = sel.layer.saturating_add(count);
    Ok(json!({ "layers": count }))
}

fn distribute_to_keyframes(doc: &mut AniDoc, _params: &Value) -> Out {
    let sel = top_level(doc)?;
    let id = doc.editor.state.timeline.clone();
    let elements = edit::remove_elements(doc.document_mut(), &id, sel.layer, sel.keyframe, &sel.indices).map_err(failed)?;
    let count = elements.len();
    let mut items = Vec::new();
    for (n, element) in elements.into_iter().enumerate() {
        let frame = sel.keyframe.saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
        let starts_here =
            doc.document().timeline(&id).and_then(|t| t.layers.get(sel.layer)).and_then(|l| l.keyframe_at(frame)).is_some_and(|k| k.index == frame);
        if !starts_here {
            edit::insert_blank_keyframe(doc.document_mut(), &id, sel.layer, frame).map_err(failed)?;
        }
        let at = edit::add_element(doc.document_mut(), &id, sel.layer, frame, element).map_err(failed)?;
        items.push(ElementRef::top_level(sel.layer, frame, at));
    }
    doc.editor.state.selection = Selection { items, scope: None };
    Ok(json!({ "keyframes": count }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Lock
// ---------------------------------------------------------------------------------------------------------------------

fn lock_selection(doc: &mut AniDoc, _params: &Value) -> Out {
    let id = doc.editor.state.timeline.clone();
    let items = doc.editor.state.selection.items.clone();
    let mut locked = 0_usize;
    for r in &items {
        if let Some(e) = element_mut(doc.document_mut(), &id, r)
            && set_locked(e, true)
        {
            locked = locked.saturating_add(1);
        }
    }
    if locked == 0 {
        return Err(CommandError::failed("nothing selected can be locked"));
    }
    doc.editor.state.selection = doc.editor.state.selection.with_items(Vec::new());
    Ok(json!({ "locked": locked }))
}

fn unlock_in(elements: &mut [Element], depth: usize) -> usize {
    let mut n = 0_usize;
    for e in elements {
        if set_locked(e, false) {
            n = n.saturating_add(1);
        }
        if let Element::Group(g) = e
            && depth < anim_scene::MAX_GROUP_DEPTH
        {
            n = n.saturating_add(unlock_in(&mut g.members, depth.saturating_add(1)));
        }
    }
    n
}

fn unlock_all(doc: &mut AniDoc, _params: &Value) -> Out {
    let id = doc.editor.state.timeline.clone();
    let tl = doc.document_mut().timeline_mut(&id).ok_or_else(|| CommandError::failed("the timeline is gone"))?;
    let mut n = 0_usize;
    for layer in &mut tl.layers {
        for frame in &mut layer.frames {
            n = n.saturating_add(unlock_in(&mut frame.elements, 0));
        }
    }
    Ok(json!({ "unlocked": n }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Transform
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformParams {
    op: String,
    scale: Option<f64>,
    degrees: Option<f64>,
}

fn selection_center(doc: &AniDoc) -> Result<Point, CommandError> {
    #[expect(clippy::redundant_closure_for_method_calls, reason = "the method reference does not fit the closure's higher-ranked lifetime")]
    let bounds: Option<Rect> = doc.editor.with_context(|ctx| ctx.selection_bounds());
    bounds.map(|b| b.center()).ok_or_else(|| CommandError::failed("nothing is selected"))
}

fn about(center: Point, linear: Affine) -> Affine {
    Affine::translate(center.to_vec2()) * linear * Affine::translate(-center.to_vec2())
}

fn transform(doc: &mut AniDoc, params: &Value) -> Out {
    let p: TransformParams = parse_params(params)?;
    if p.op == "remove" {
        return remove_transform(doc);
    }
    let center = selection_center(doc)?;
    let linear = match p.op.as_str() {
        "rotate_90_cw" => Affine::rotate(std::f64::consts::FRAC_PI_2),
        "rotate_90_ccw" => Affine::rotate(-std::f64::consts::FRAC_PI_2),
        "scale_rotate" => {
            let scale = p.scale.unwrap_or(100.0) / 100.0;
            let degrees = p.degrees.unwrap_or(0.0);
            if !(scale.is_finite() && scale > 0.0 && degrees.is_finite()) {
                return Err(CommandError::invalid_params("scale must be positive and degrees finite"));
            }
            Affine::rotate(degrees.to_radians()) * Affine::scale(scale)
        }
        other => return Err(CommandError::invalid_params(format!("unknown transform {other:?}"))),
    };
    doc.editor.run_commands(&[ToolCommand::TransformSelection { matrix: about(center, linear) }]).map_err(failed)?;
    Ok(json!({ "op": p.op }))
}

fn remove_transform(doc: &mut AniDoc) -> Out {
    let id = doc.editor.state.timeline.clone();
    let items = doc.editor.state.selection.items.clone();
    let mut changed = 0_usize;
    for r in &items {
        let Some(e) = element_mut(doc.document_mut(), &id, r) else { continue };
        let Some(m) = e.matrix() else { continue };
        let [_, _, _, _, tx, ty] = m.as_coeffs();
        let plain = Affine::translate((tx, ty));
        if plain != m {
            e.set_matrix(plain);
            changed = changed.saturating_add(1);
        }
    }
    if changed == 0 {
        return Err(CommandError::failed("the selected objects have no transformation to remove"));
    }
    Ok(json!({ "objects": changed }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Paste
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasteParams {
    elements: Vec<Value>,
    position: Option<String>,
    offset: Option<[f64; 2]>,
    #[serde(default)]
    as_bitmap: bool,
}

fn paste(doc: &mut AniDoc, params: &Value) -> Out {
    let p: PasteParams = parse_params(params)?;
    let mut elements: Vec<Element> = Vec::with_capacity(p.elements.len());
    for value in p.elements {
        elements.push(serde_json::from_value(value).map_err(|e| CommandError::invalid_params(format!("not an element: {e}")))?);
    }
    if elements.is_empty() {
        return Err(CommandError::failed("there is nothing to paste"));
    }
    let id = doc.editor.state.timeline.clone();
    let (layer, frame) = (doc.editor.state.layer, doc.editor.state.frame);
    let shift = match p.position.as_deref() {
        None | Some("offset") => p.offset.map_or(anim_core::Vec2::new(12.0, 12.0), |o| anim_core::Vec2::new(o[0], o[1])),
        Some("in_place") => anim_core::Vec2::ZERO,
        Some("center") => {
            let bounds = elements_bounds(doc, &elements)?;
            let (w, h) = (doc.document().width, doc.document().height);
            anim_core::Vec2::new(w / 2.0 - bounds.center().x, h / 2.0 - bounds.center().y)
        }
        Some(other) => return Err(CommandError::invalid_params(format!("unknown position {other:?}"))),
    };
    for e in &mut elements {
        compose(Affine::translate(shift), e);
    }
    let mut placed = Vec::new();
    if p.as_bitmap {
        let at = super::bitmap_ops::paste_as_bitmap(doc, &id, layer, frame, &elements)?;
        placed.push(at);
    } else {
        for e in elements {
            placed.push(edit::add_element(doc.document_mut(), &id, layer, frame, e).map_err(failed)?);
        }
    }
    let keyframe = doc.document().timeline(&id).and_then(|t| t.layers.get(layer)).and_then(|l| l.keyframe_at(frame)).map_or(0, |k| k.index);
    doc.editor.state.selection = Selection { items: placed.iter().map(|&i| ElementRef::top_level(layer, keyframe, i)).collect(), scope: None };
    Ok(json!({ "pasted": placed.len() }))
}

fn elements_bounds(doc: &AniDoc, elements: &[Element]) -> Result<Rect, CommandError> {
    let provider = anicraft_tools::LibraryBounds::new(&doc.document().library);
    elements
        .iter()
        .filter_map(|e| anicraft_tools::hit::bounds_in_parent(&provider, e, 0))
        .reduce(|a, b| a.union(b))
        .ok_or_else(|| CommandError::failed("the pasted objects have no extent"))
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
    use anim_session::Session;

    use super::*;
    use crate::mcp::registry;

    fn session() -> Session<AniDoc> {
        Session::new(AniDoc::default(), registry())
    }

    fn keyframe_elements(s: &Session<AniDoc>, layer: usize) -> Vec<Element> {
        s.doc().editor.document.scenes[0].layers[layer].keyframe_at(0).unwrap().elements.clone()
    }

    fn rects(s: &mut Session<AniDoc>, n: u32) {
        for i in 0..n {
            s.execute("draw.rectangle", &json!({ "x": i * 60, "y": 10, "width": 50, "height": 50, "fill": "#336699", "object_drawing": true })).unwrap();
        }
        s.execute("select.all", &json!({})).unwrap();
    }

    #[test]
    fn distributing_to_layers_makes_a_layer_per_object() {
        let mut s = session();
        rects(&mut s, 3);
        let layers_before = s.doc().editor.document.scenes[0].layers.len();
        s.execute("modify.distribute_to_layers", &json!({})).unwrap();
        let layers = &s.doc().editor.document.scenes[0].layers;
        assert_eq!(layers.len(), layers_before + 3);
        let counts: Vec<usize> = (0..layers.len()).map(|i| keyframe_elements(&s, i).len()).collect();
        assert_eq!(counts, vec![1, 1, 1, 0], "{counts:?}");
        assert_eq!(s.doc().editor.state.selection.items.len(), 3);
        s.undo();
        assert_eq!(s.doc().editor.document.scenes[0].layers.len(), layers_before);
        assert_eq!(keyframe_elements(&s, 0).len(), 3);
    }

    #[test]
    fn distributing_to_keyframes_spreads_the_objects_over_frames() {
        let mut s = session();
        rects(&mut s, 3);
        s.execute("modify.distribute_to_keyframes", &json!({})).unwrap();
        let layer = &s.doc().editor.document.scenes[0].layers[0];
        let per_frame: Vec<usize> = (0..3).map(|f| layer.keyframe_at(f).unwrap().elements.len()).collect();
        assert_eq!(per_frame, vec![1, 1, 1]);
        assert!(layer.keyframe_at(2).unwrap().index == 2);
    }

    #[test]
    fn locked_objects_cannot_be_picked_until_unlocked() {
        let mut s = session();
        rects(&mut s, 1);
        s.execute("modify.lock", &json!({})).unwrap();
        assert!(s.doc().editor.state.selection.items.is_empty());
        s.execute("select.all", &json!({})).unwrap();
        assert!(s.doc().editor.state.selection.items.is_empty(), "a locked object is not selectable");
        let report = s.execute("modify.unlock_all", &json!({})).unwrap();
        assert_eq!(report["unlocked"], 1);
        s.execute("select.all", &json!({})).unwrap();
        assert_eq!(s.doc().editor.state.selection.items.len(), 1);
    }

    #[test]
    fn rotating_ninety_degrees_swaps_the_extent() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 100, "height": 40, "fill": "#ff0000", "object_drawing": true })).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        s.execute("modify.transform", &json!({ "op": "rotate_90_cw" })).unwrap();
        let bounds = s.doc().editor.with_context(|c| c.selection_bounds()).unwrap();
        assert!((bounds.width() - 40.0).abs() < 2.0 && (bounds.height() - 100.0).abs() < 2.0, "{bounds:?}");
        assert!((bounds.center().x - 50.0).abs() < 1.0 && (bounds.center().y - 20.0).abs() < 1.0, "turned about its centre");
        s.execute("modify.transform", &json!({ "op": "scale_rotate", "scale": 200, "degrees": 90 })).unwrap();
        let bounds = s.doc().editor.with_context(|c| c.selection_bounds()).unwrap();
        assert!((bounds.width() - 200.0).abs() < 4.0, "{bounds:?}");
        s.execute("modify.transform", &json!({ "op": "remove" })).unwrap();
        let bounds = s.doc().editor.with_context(|c| c.selection_bounds()).unwrap();
        assert!((bounds.width() - 100.0).abs() < 2.0 && (bounds.height() - 40.0).abs() < 2.0, "{bounds:?}");
        assert!(s.execute("modify.transform", &json!({ "op": "tilt" })).is_err());
    }

    #[test]
    fn groups_nest_ungroup_and_break_apart() {
        let mut s = session();
        rects(&mut s, 2);
        s.execute("modify.group", &json!({})).unwrap();
        assert_eq!(keyframe_elements(&s, 0).len(), 1);
        // Group a group with a third object, then take both apart again.
        s.execute("draw.rectangle", &json!({ "x": 200, "y": 10, "width": 50, "height": 50, "object_drawing": true })).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        s.execute("modify.group", &json!({})).unwrap();
        let top = keyframe_elements(&s, 0);
        assert_eq!(top.len(), 1);
        let Element::Group(outer) = &top[0] else { panic!("group") };
        assert_eq!(outer.members.len(), 2);
        s.execute("modify.ungroup", &json!({})).unwrap();
        assert_eq!(keyframe_elements(&s, 0).len(), 2);
        // A group breaks apart into its members; a drawing object into the keyframe's merge-mode shape.
        s.execute("select.all", &json!({})).unwrap();
        s.execute("modify.break_apart", &json!({})).unwrap();
        let after = keyframe_elements(&s, 0);
        assert_eq!(after.len(), 3);
        assert!(matches!(&after[0], Element::Shape(sh) if sh.is_drawing_object));
        assert!(matches!(&after[2], Element::Shape(sh) if !sh.is_drawing_object), "{after:?}");
        s.execute("select.all", &json!({})).unwrap();
        s.execute("modify.break_apart", &json!({})).unwrap();
        let merged = keyframe_elements(&s, 0);
        assert_eq!(merged.len(), 1, "everything merged into one shape");
        assert!(matches!(&merged[0], Element::Shape(sh) if !sh.is_drawing_object));
        assert!(s.execute("modify.ungroup", &json!({})).is_err(), "no group is selected");
    }

    #[test]
    fn pasting_places_copies_in_place_or_offset() {
        let mut s = session();
        rects(&mut s, 1);
        let copied = s.execute("edit.copy", &json!({})).unwrap();
        let elements = copied["elements"].clone();
        s.execute("edit.paste", &json!({ "elements": elements, "position": "in_place" })).unwrap();
        let both = keyframe_elements(&s, 0);
        assert_eq!(both.len(), 2);
        assert_eq!(both[0].matrix(), both[1].matrix());
        s.execute("edit.paste", &json!({ "elements": copied["elements"], "offset": [30, 5] })).unwrap();
        let three = keyframe_elements(&s, 0);
        let base = both[0].matrix().unwrap().as_coeffs();
        let moved = three[2].matrix().unwrap().as_coeffs();
        assert!((moved[4] - base[4] - 30.0).abs() < 1e-6 && (moved[5] - base[5] - 5.0).abs() < 1e-6);
        s.execute("edit.paste", &json!({ "elements": copied["elements"], "position": "center" })).unwrap();
        assert_eq!(keyframe_elements(&s, 0).len(), 4);
        assert!(s.execute("edit.paste", &json!({ "elements": [] })).is_err());
    }
}
