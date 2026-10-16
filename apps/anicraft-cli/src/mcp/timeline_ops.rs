//! Timeline operations: scenes, layer types and properties, the frame clipboard, reversing frames, and the editing
//! position (symbols, breadcrumbs, Edit Multiple Frames, auto keyframe).
//!
//! The same conventions as [`super::commands`]: layers are 0-based, frames are 1-based, `scene` defaults to the
//! timeline the editor is in. Every command is one undo step (the editor-position ones change no document content and
//! make none) and a failure changes nothing.

use anim_core::Rgba8;
use anim_scene::edit::{self, MAX_FRAMES};
use anim_scene::timeline::{LayerKind, TweenType};
use anim_scene::{Frame, Layer, SceneError, Timeline, TimelineId};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::commands::{parse_color, timeline, zero_based};
use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::failed(e.to_string())
}

/// The commands of this module.
#[must_use]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
    let mut specs = scene_commands();
    specs.extend(layer_commands());
    specs.extend(frame_commands());
    specs.extend(position_commands());
    specs
}

/// Registers [`all`] into `registry`.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(all()).expect("timeline operation ids are unique");
}

fn scene_commands() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("scene.move", "Move Scene", scene_move)
            .describe("Moves a scene to position `to` in the playback order (0-based). The open scene stays open.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["scene","to"],"properties":{"scene":{"type":"integer","minimum":0},"to":{"type":"integer","minimum":0}}}"#),
        CommandSpec::new("scene.duplicate", "Duplicate Scene", scene_duplicate)
            .describe("Copies a scene with all its layers and frames; the copy goes right after it, named `name` or `<scene> copy`.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["scene"],"properties":{"scene":{"type":"integer","minimum":0},"name":{"type":"string","minLength":1}}}"#),
    ]
}

fn layer_commands() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("layer.set_kind", "Layer Type", set_kind)
            .describe("Converts a layer to `normal`, `guide`, `mask` or `folder`. A mask layer masks the normal layer directly below it; layers inside a folder, mask or guide that stops being one are released to the level above. A layer with content cannot become a folder.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer","kind"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"kind":{"enum":["normal","guide","mask","folder"]}}}"#)),
        CommandSpec::new("layer.add_motion_guide", "Add Classic Motion Guide", add_motion_guide)
            .in_menu(&["Insert", "Timeline"])
            .describe("Adds a guide layer named `Guide: <layer>` directly above the layer and makes the layer guided by it.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0}}}"#)),
        CommandSpec::new("layer.set_parent", "Set Layer Parent", set_parent)
            .describe("Parenting view: puts the layer inside the folder, mask or guide layer `parent` (which must be above it), or at the top level when `parent` is omitted.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"parent":{"type":"integer","minimum":0}}}"#)),
        CommandSpec::new("layer.set_properties", "Layer Properties", set_properties)
            .in_menu(&["Modify", "Timeline"])
            .describe("The Layer Properties dialog: `name`, `kind` (normal, guide, mask, folder), outline `color` (#rrggbb), `height` (1, 2 or 3 times the normal row height), `visible`, `locked`, `outline`. Omitted fields stay as they are.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0}"#, r##","name":{"type":"string","minLength":1},"kind":{"enum":["normal","guide","mask","folder"]},"color":{"type":"string","description":"#rrggbb"},"height":{"enum":[1,2,3]},"visible":{"type":"boolean"},"locked":{"type":"boolean"},"outline":{"type":"boolean"}}}"##)),
    ]
}

fn frame_commands() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("timeline.copy_frames", "Copy Frames", copy_frames)
            .in_menu(&["Edit", "Timeline"])
            .transient()
            .describe("Copies the frames `from`..=`to` of the layers and returns them as a `clip` that `timeline_paste_frames` accepts.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["from","to"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layers":{"type":"array","items":{"type":"integer","minimum":0}},"from":{"type":"integer","minimum":1},"to":{"type":"integer","minimum":1}}}"#)),
        CommandSpec::new("timeline.cut_frames", "Cut Frames", cut_frames)
            .in_menu(&["Edit", "Timeline"])
            .describe("Copies the frames `from`..=`to` of the layers (returned as `clip`) and removes them; later frames move left.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["from","to"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layers":{"type":"array","items":{"type":"integer","minimum":0}},"from":{"type":"integer","minimum":1},"to":{"type":"integer","minimum":1}}}"#)),
        CommandSpec::new("timeline.paste_frames", "Paste Frames", paste_frames)
            .in_menu(&["Edit", "Timeline"])
            .describe("Pastes a `clip` from `timeline_copy_frames` at `frame` of `layer` (the clip's first row) and the layers below it. By default the frames already there move right; with `overwrite` they are replaced.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer","frame","clip"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1},"overwrite":{"type":"boolean","default":false},"clip":{"type":"object"}}}"#)),
        CommandSpec::new("timeline.clear_frames", "Clear Frames", clear_frames)
            .in_menu(&["Edit", "Timeline"])
            .describe("Empties the frames `from`..=`to` of the layers: they become one blank keyframe; the timeline keeps its length.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["from","to"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layers":{"type":"array","items":{"type":"integer","minimum":0}},"from":{"type":"integer","minimum":1},"to":{"type":"integer","minimum":1}}}"#)),
        CommandSpec::new("timeline.reverse_frames", "Reverse Frames", reverse_frames)
            .in_menu(&["Modify", "Timeline"])
            .describe("Reverses the order of the keyframes between `from` and `to` on the layers. Tweens run the other way (ease is mirrored).")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["from","to"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layers":{"type":"array","items":{"type":"integer","minimum":0}},"from":{"type":"integer","minimum":1},"to":{"type":"integer","minimum":1}}}"#)),
        CommandSpec::new("timeline.convert_to_keyframes", "Convert to Keyframes", convert_to_keyframes)
            .in_menu(&["Modify", "Timeline"])
            .describe("Makes every frame `from`..=`to` of the layers a keyframe (a copy of what shows there, or empty with `blank`).")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["from","to"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layers":{"type":"array","items":{"type":"integer","minimum":0}},"from":{"type":"integer","minimum":1},"to":{"type":"integer","minimum":1},"blank":{"type":"boolean","default":false}}}"#)),
    ]
}

fn position_commands() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("edit.enter_symbol", "Edit Symbol", enter_symbol)
            .transient()
            .describe("Edits a library symbol on a stage of its own (Edit > Edit Symbols); `in_place` shows it in the context of the timeline it is entered from instead, placed by `matrix` [a,b,c,d,tx,ty].")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["name"],"properties":{"name":{"type":"string"},"in_place":{"type":"boolean","default":false},"matrix":{"type":"array","items":{"type":"number"},"minItems":6,"maxItems":6}}}"#),
        CommandSpec::new("edit.exit_to", "Exit to Level", exit_to)
            .transient()
            .describe("Leaves symbols until `depth` levels remain on the edit bar's breadcrumb (0: back to the scene).")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["depth"],"properties":{"depth":{"type":"integer","minimum":0}}}"#),
        CommandSpec::new("editor.set_edit_range", "Edit Multiple Frames", set_edit_range)
            .transient()
            .describe("Edit Multiple Frames: while a range is set the selection tools pick and edit the keyframes of every frame from `from` to `to` (1-based, inclusive) at once. Without them, only the keyframes under the playhead are editable.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"properties":{"from":{"type":"integer","minimum":1},"to":{"type":"integer","minimum":1}}}"#),
        CommandSpec::new("editor.set_auto_keyframe", "Auto Keyframe", set_auto_keyframe)
            .transient()
            .describe("With auto keyframe on, drawing or moving something on a held frame (inside a span) first makes that frame a keyframe, so only that frame changes.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["enabled"],"properties":{"enabled":{"type":"boolean"}}}"#),
    ]
}

// ---------------------------------------------------------------------------------------------------------------
// Scenes
// ---------------------------------------------------------------------------------------------------------------

/// Points the editor at the scene a scene operation moved: `map` gives a scene's new index, `None` when it is gone.
/// The document's own open scene is the caller's business (`edit::add_scene` and `edit::remove_scene` fix it).
fn remap_scenes(doc: &mut AniDoc, map: impl Fn(usize) -> Option<usize>) {
    let last = doc.editor.document.scenes.len().saturating_sub(1);
    let mut gone = false;
    if let TimelineId::Scene(i) = doc.editor.state.timeline {
        match map(i) {
            Some(j) => doc.editor.state.timeline = TimelineId::Scene(j.min(last)),
            None => gone = true,
        }
    }
    for (timeline, ..) in &mut doc.editor.state.timeline_stack {
        if let TimelineId::Scene(i) = timeline {
            *i = map(*i).unwrap_or(0).min(last);
        }
    }
    if gone {
        let fallback = doc.editor.document.current_timeline.min(last);
        let state = &mut doc.editor.state;
        state.timeline = TimelineId::Scene(fallback);
        state.timeline_stack.clear();
        state.in_place_stack.clear();
        state.in_place = None;
        state.edit_range = None;
        state.layer = 0;
        state.frame = 0;
        state.selection = anicraft_tools::Selection::empty();
    }
}

/// After `edit::add_scene` put a scene at `at`: the editor keeps editing the scene it was in.
pub fn scene_added(doc: &mut AniDoc, at: usize) {
    remap_scenes(doc, |i| Some(if i >= at { i.saturating_add(1) } else { i }));
}

/// After `edit::remove_scene` removed the scene at `removed`: the editor follows its scene, or opens the document's.
pub fn scene_removed(doc: &mut AniDoc, removed: usize) {
    remap_scenes(doc, |i| match i.cmp(&removed) {
        std::cmp::Ordering::Less => Some(i),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(i.saturating_sub(1)),
    });
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneMove {
    scene: usize,
    to: usize,
}

fn scene_move(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SceneMove = parse_params(params)?;
    let count = doc.editor.document.scenes.len();
    if p.scene >= count {
        return Err(CommandError::failed(format!("no scene {}; the document has {count}", p.scene)));
    }
    let to = p.to.min(count.saturating_sub(1));
    let moved = doc.editor.document.scenes.remove(p.scene);
    doc.editor.document.scenes.insert(to, moved);
    let from = p.scene;
    let map = |i: usize| {
        if i == from {
            to
        } else if from < i && i <= to {
            i.saturating_sub(1)
        } else if to <= i && i < from {
            i.saturating_add(1)
        } else {
            i
        }
    };
    remap_scenes(doc, |i| Some(map(i)));
    doc.editor.document.current_timeline = map(doc.editor.document.current_timeline);
    Ok(json!({ "scene": to }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneDuplicate {
    scene: usize,
    name: Option<String>,
}

fn scene_duplicate(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SceneDuplicate = parse_params(params)?;
    let source = doc.editor.document.scenes.get(p.scene).ok_or_else(|| CommandError::failed(format!("no scene {}", p.scene)))?.clone();
    let base = p.name.unwrap_or_else(|| format!("{} copy", source.name));
    let mut name = base.clone();
    let mut n = 1_u32;
    while doc.editor.document.scene_index(&name).is_some() {
        n = n.saturating_add(1);
        name = format!("{base} {n}");
    }
    let mut copy = source;
    copy.name.clone_from(&name);
    let at = p.scene.saturating_add(1);
    doc.editor.document.scenes.insert(at, copy);
    scene_added(doc, at);
    if doc.editor.document.current_timeline >= at {
        doc.editor.document.current_timeline = doc.editor.document.current_timeline.saturating_add(1);
    }
    Ok(json!({ "scene": at, "name": name }))
}

// ---------------------------------------------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------------------------------------------

/// The layer types the Layer Properties dialog offers.
fn kind_named(name: &str) -> Result<LayerKind, CommandError> {
    match name {
        "normal" => Ok(LayerKind::Normal),
        "guide" => Ok(LayerKind::Guide),
        "mask" => Ok(LayerKind::Mask),
        "folder" => Ok(LayerKind::Folder),
        other => Err(CommandError::invalid_params(format!("unknown layer type {other:?}; use normal, guide, mask or folder"))),
    }
}

/// The attribute XFL keeps the layer height (a multiplier of the normal row height) in.
const HEIGHT_ATTR: &str = "heightMultiplier";

/// A layer's row height as a multiple of the normal height: 1, 2 or 3.
#[must_use]
pub fn layer_height(layer: &Layer) -> u8 {
    layer.extra.attr(HEIGHT_ATTR).and_then(|v| v.trim().parse::<u8>().ok()).map_or(1, |h| h.clamp(1, 3))
}

fn set_layer_height(layer: &mut Layer, height: u8) {
    if height <= 1 {
        layer.extra.remove_attr(HEIGHT_ATTR);
    } else {
        layer.extra.set_attr(HEIGHT_ATTR, height.min(3).to_string());
    }
}

/// Converts layer `index` to `kind`, keeping the folder / mask / guide relations consistent.
///
/// # Errors
/// An unknown layer, a camera layer, a layer with content turned into a folder, or a folder put under a mask or
/// guide.
pub fn convert_layer(tl: &mut Timeline, index: usize, kind: LayerKind) -> Result<(), SceneError> {
    let count = tl.layers.len();
    let layer = tl.layers.get(index).ok_or(SceneError::NoSuchLayer { layer: index, count })?;
    let old = layer.kind;
    if old == LayerKind::Camera || !matches!(kind, LayerKind::Normal | LayerKind::Guide | LayerKind::Mask | LayerKind::Folder) {
        return Err(SceneError::InvalidParent("a camera layer keeps its type"));
    }
    if old == kind {
        return Ok(());
    }
    if kind == LayerKind::Folder && layer.frames.iter().any(|f| !f.elements.is_empty()) {
        return Err(SceneError::InvalidParent("a layer with content cannot become a folder; move its content to another layer first"));
    }
    let outer = tl.parent_of(index);
    let children = tl.children_of(index);
    if matches!(kind, LayerKind::Mask | LayerKind::Guide) && children.iter().any(|&c| tl.layers.get(c).is_some_and(|l| l.kind == LayerKind::Folder)) {
        return Err(SceneError::InvalidParent("a folder inside cannot be masked or guided"));
    }
    // What the layer held together stops being held when it is no longer a container, or holds differently.
    if matches!(old, LayerKind::Folder | LayerKind::Mask | LayerKind::Guide) && kind == LayerKind::Normal {
        for &c in &children {
            if let Some(child) = tl.layers.get_mut(c) {
                child.parent_layer_index = outer;
            }
        }
    }
    // A mask, guide or folder is not itself masked or guided.
    let under_mask_or_guide = outer.and_then(|p| tl.layers.get(p)).is_some_and(|p| matches!(p.kind, LayerKind::Mask | LayerKind::Guide));
    if let Some(l) = tl.layers.get_mut(index) {
        l.kind = kind;
        if kind != LayerKind::Normal && under_mask_or_guide {
            l.parent_layer_index = None;
        }
    }
    // A new mask masks the normal layer below it.
    if kind == LayerKind::Mask {
        let below = index.saturating_add(1);
        let same_level = tl.parent_of(below) == tl.parent_of(index);
        if tl.layers.get(below).is_some_and(|l| l.kind == LayerKind::Normal) && same_level {
            edit_parent(tl, below, index)?;
        }
    }
    Ok(())
}

fn edit_parent(tl: &mut Timeline, layer: usize, parent: usize) -> Result<(), SceneError> {
    let count = tl.layers.len();
    if parent >= layer {
        return Err(SceneError::InvalidParent("the parent must be above the layer"));
    }
    tl.layers.get_mut(layer).ok_or(SceneError::NoSuchLayer { layer, count })?.parent_layer_index = Some(parent);
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetKind {
    scene: Option<usize>,
    layer: usize,
    kind: String,
}

fn set_kind(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SetKind = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let kind = kind_named(&p.kind)?;
    let tl = doc.editor.document.timeline_mut(&id).ok_or_else(|| CommandError::failed("no such timeline"))?;
    convert_layer(tl, p.layer, kind).map_err(failed)?;
    Ok(json!({ "layer": p.layer, "kind": p.kind }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerOnly {
    scene: Option<usize>,
    layer: usize,
}

fn add_motion_guide(doc: &mut AniDoc, params: &Value) -> Out {
    let p: LayerOnly = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let document = &mut doc.editor.document;
    let tl = document.timeline(&id).ok_or_else(|| CommandError::failed("no such timeline"))?;
    let target = tl.layers.get(p.layer).ok_or_else(|| CommandError::failed(format!("no layer {}", p.layer)))?;
    if !matches!(tl.resolved_kind(p.layer), Some(LayerKind::Normal | LayerKind::Guided | LayerKind::Masked)) {
        return Err(CommandError::failed("only a normal layer can get a motion guide"));
    }
    let name = format!("Guide: {}", target.name);
    let folder = tl.parent_of(p.layer).filter(|&f| tl.layers.get(f).is_some_and(|l| l.kind == LayerKind::Folder));
    let guide = edit::add_layer(document, &id, p.layer, Some(&name), LayerKind::Guide).map_err(failed)?;
    let guided = guide.saturating_add(1);
    edit::set_layer_parent(document, &id, guide, folder).map_err(failed)?;
    edit::set_layer_parent(document, &id, guided, Some(guide)).map_err(failed)?;
    Ok(json!({ "guide": guide, "layer": guided }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetParent {
    scene: Option<usize>,
    layer: usize,
    parent: Option<usize>,
}

fn set_parent(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SetParent = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    edit::set_layer_parent(&mut doc.editor.document, &id, p.layer, p.parent).map_err(failed)?;
    Ok(json!({ "layer": p.layer, "parent": p.parent }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetProperties {
    scene: Option<usize>,
    layer: usize,
    name: Option<String>,
    kind: Option<String>,
    color: Option<String>,
    height: Option<u8>,
    visible: Option<bool>,
    locked: Option<bool>,
    outline: Option<bool>,
}

fn set_properties(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SetProperties = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let color: Option<Rgba8> = p.color.as_deref().map(parse_color).transpose()?;
    if p.height.is_some_and(|h| !(1..=3).contains(&h)) {
        return Err(CommandError::invalid_params("height is 1, 2 or 3"));
    }
    if let Some(name) = &p.name {
        edit::rename_layer(&mut doc.editor.document, &id, p.layer, name).map_err(failed)?;
    }
    let tl = doc.editor.document.timeline_mut(&id).ok_or_else(|| CommandError::failed("no such timeline"))?;
    if let Some(kind) = p.kind.as_deref() {
        convert_layer(tl, p.layer, kind_named(kind)?).map_err(failed)?;
    }
    let count = tl.layers.len();
    let layer = tl.layers.get_mut(p.layer).ok_or_else(|| failed(SceneError::NoSuchLayer { layer: p.layer, count }))?;
    if let Some(c) = color {
        layer.color = c;
    }
    if let Some(h) = p.height {
        set_layer_height(layer, h);
    }
    if let Some(v) = p.visible {
        layer.visible = v;
    }
    if let Some(v) = p.locked {
        layer.locked = v;
    }
    if let Some(v) = p.outline {
        layer.outline = v;
    }
    Ok(json!({ "layer": p.layer, "name": layer.name, "height": layer_height(layer) }))
}

// ---------------------------------------------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------------------------------------------

/// Splits the keyframe that covers `at` (if it starts before) into two keyframes at `at`. The tail repeats the
/// content; a tween on the head would no longer lead anywhere sensible and is dropped.
fn split_at(l: &mut Layer, at: u32) {
    l.sort_frames();
    let Some(pos) = l.keyframe_position_at(at) else { return };
    let Some(k) = l.frames.get(pos) else { return };
    if k.index >= at {
        return;
    }
    let tail = Frame::new(at, k.end().saturating_sub(at), k.elements.clone());
    if let Some(head) = l.frames.get_mut(pos) {
        head.duration = at.saturating_sub(head.index);
        head.clear_tween();
    }
    l.frames.insert(pos.saturating_add(1), tail);
}

/// Removes everything on frames `from..end` (exclusive end), leaving a gap and trimming the spans around it.
fn clear_range(l: &mut Layer, from: u32, end: u32) {
    split_at(l, from);
    split_at(l, end);
    l.frames.retain(|f| f.index < from || f.index >= end);
}

/// Drops tweens that lost the keyframe they tween to.
fn drop_dangling_tweens(l: &mut Layer) {
    l.sort_frames();
    if let Some(last) = l.frames.last_mut()
        && last.is_tweened()
        && last.tween_type != TweenType::MotionObject
    {
        last.clear_tween();
    }
}

/// The frames `from..end` of a layer as keyframes whose indices count from `from`. A keyframe that began before
/// `from` is copied from `from` on, without its label, sound or script (they belong to the original keyframe).
fn copy_range(l: &Layer, from: u32, end: u32) -> Vec<Frame> {
    let mut out = Vec::new();
    for f in &l.frames {
        let start = f.index.max(from);
        let stop = f.end().min(end);
        if start >= stop {
            continue;
        }
        let mut copy = f.clone();
        copy.index = start.saturating_sub(from);
        copy.duration = stop.saturating_sub(start);
        if f.index < from {
            copy.label = None;
            copy.label_type = None;
            copy.sound = None;
            copy.actionscript = None;
        }
        if f.end() > end && copy.is_tweened() {
            copy.clear_tween();
        }
        out.push(copy);
    }
    out
}

/// Puts the clip frames on `l` starting at `at`, replacing what is there (`width` frames of it).
fn paste_over(l: &mut Layer, at: u32, width: u32, clip: &[Frame]) {
    clear_range(l, at, at.saturating_add(width));
    place(l, at, clip);
}

/// Puts the clip frames on `l` starting at `at`, pushing the frames from `at` on right by `width`.
fn paste_inserting(l: &mut Layer, at: u32, width: u32, clip: &[Frame]) {
    split_at(l, at);
    for f in l.frames.iter_mut().filter(|f| f.index >= at) {
        f.index = f.index.saturating_add(width);
    }
    place(l, at, clip);
}

fn place(l: &mut Layer, at: u32, clip: &[Frame]) {
    for f in clip {
        let mut copy = f.clone();
        copy.index = copy.index.saturating_add(at);
        l.frames.push(copy);
    }
    l.sort_frames();
    drop_dangling_tweens(l);
}

/// Copies the tween settings of `from` to `to` for a tween that runs the other way: ease mirrored, shape hints
/// swapped.
fn copy_reversed_tween(from: &Frame, to: &mut Frame) {
    to.tween_type = from.tween_type;
    to.key_mode = from.key_mode;
    to.motion_tween_rotate = match from.motion_tween_rotate {
        anim_scene::timeline::TweenRotate::Clockwise => anim_scene::timeline::TweenRotate::CounterClockwise,
        anim_scene::timeline::TweenRotate::CounterClockwise => anim_scene::timeline::TweenRotate::Clockwise,
        other => other,
    };
    to.motion_tween_rotate_times = from.motion_tween_rotate_times;
    to.motion_tween_scale = from.motion_tween_scale;
    to.motion_tween_snap = from.motion_tween_snap;
    to.motion_tween_orient_to_path = from.motion_tween_orient_to_path;
    to.motion_tween_sync = from.motion_tween_sync;
    to.acceleration = from.acceleration.saturating_neg();
    to.ease_method_name.clone_from(&from.ease_method_name);
    to.eases.clone_from(&from.eases);
    to.shape_hints = from.shape_hints.iter().map(|h| anim_scene::timeline::ShapeHint { id: h.id, start: h.end, end: h.start }).collect();
    to.morph_shape_xml = None;
    to.motion_object_xml.clone_from(&from.motion_object_xml);
}

/// Reverses the keyframes of frames `from..end` of a layer.
fn reverse_range(l: &mut Layer, from: u32, end: u32) {
    let clip = copy_range(l, from, end);
    if clip.is_empty() {
        return;
    }
    let width = end.saturating_sub(from);
    clear_range(l, from, end);
    let mut mirrored: Vec<Frame> = clip
        .iter()
        .map(|f| {
            let mut m = f.clone();
            m.index = width.saturating_sub(f.index.saturating_add(f.duration));
            m.clear_tween();
            m
        })
        .collect();
    // A tween from keyframe A to the next keyframe B now starts at B and ends at A.
    for (i, pair) in clip.windows(2).enumerate() {
        if let [a, b] = pair
            && a.is_tweened()
            && a.tween_type != TweenType::MotionObject
            && a.end() == b.index
            && let Some(target) = mirrored.get_mut(i.saturating_add(1))
        {
            copy_reversed_tween(a, target);
        }
    }
    place(l, from, &mirrored);
}

/// The frame clipboard: the frames of some layers, top to bottom, with indices counted from the first copied frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameClip {
    /// How many frames the copied range was wide.
    pub width: u32,
    /// One entry per copied layer.
    pub rows: Vec<Vec<Frame>>,
}

struct Range {
    id: TimelineId,
    layers: Vec<usize>,
    from: u32,
    end: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RangeParams {
    scene: Option<usize>,
    layers: Option<Vec<usize>>,
    from: u32,
    to: u32,
    #[serde(default)]
    blank: bool,
}

fn resolve_range(doc: &AniDoc, p: &RangeParams) -> Result<Range, CommandError> {
    let id = timeline(doc, p.scene)?;
    let (from, to) = (zero_based(p.from)?, zero_based(p.to)?);
    if to < from {
        return Err(CommandError::invalid_params("`to` is before `from`"));
    }
    if to >= MAX_FRAMES {
        return Err(CommandError::failed("frame limit exceeded"));
    }
    let tl = doc.editor.document.timeline(&id).ok_or_else(|| CommandError::failed("no such timeline"))?;
    let mut layers = p.layers.clone().unwrap_or_else(|| vec![doc.editor.state.layer]);
    layers.sort_unstable();
    layers.dedup();
    if layers.is_empty() {
        return Err(CommandError::invalid_params("no layers"));
    }
    if let Some(bad) = layers.iter().find(|&&l| l >= tl.layers.len()) {
        return Err(CommandError::failed(format!("no layer {bad}; the timeline has {}", tl.layers.len())));
    }
    Ok(Range { id, layers, from, end: to.saturating_add(1) })
}

fn layers_mut<'a>(doc: &'a mut AniDoc, range: &Range) -> Result<Vec<&'a mut Layer>, CommandError> {
    let tl = doc.editor.document.timeline_mut(&range.id).ok_or_else(|| CommandError::failed("no such timeline"))?;
    Ok(tl.layers.iter_mut().enumerate().filter(|(i, _)| range.layers.contains(i)).map(|(_, l)| l).collect())
}

fn clip_of(doc: &AniDoc, range: &Range) -> Result<FrameClip, CommandError> {
    let tl = doc.editor.document.timeline(&range.id).ok_or_else(|| CommandError::failed("no such timeline"))?;
    let rows = range.layers.iter().filter_map(|&i| tl.layers.get(i)).map(|l| copy_range(l, range.from, range.end)).collect();
    Ok(FrameClip { width: range.end.saturating_sub(range.from), rows })
}

fn copy_frames(doc: &mut AniDoc, params: &Value) -> Out {
    let p: RangeParams = parse_params(params)?;
    let range = resolve_range(doc, &p)?;
    let clip = clip_of(doc, &range)?;
    Ok(json!({ "clip": clip }))
}

fn cut_frames(doc: &mut AniDoc, params: &Value) -> Out {
    let p: RangeParams = parse_params(params)?;
    let range = resolve_range(doc, &p)?;
    let clip = clip_of(doc, &range)?;
    let count = range.end.saturating_sub(range.from);
    for &layer in &range.layers {
        edit::remove_frames(&mut doc.editor.document, &range.id, layer, range.from, count).map_err(failed)?;
    }
    Ok(json!({ "clip": clip }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasteParams {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    #[serde(default)]
    overwrite: bool,
    clip: FrameClip,
}

fn paste_frames(doc: &mut AniDoc, params: &Value) -> Out {
    let p: PasteParams = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let at = zero_based(p.frame)?;
    if at.saturating_add(p.clip.width) >= MAX_FRAMES {
        return Err(CommandError::failed("frame limit exceeded"));
    }
    let tl = doc.editor.document.timeline_mut(&id).ok_or_else(|| CommandError::failed("no such timeline"))?;
    let needed = p.layer.saturating_add(p.clip.rows.len());
    if needed > tl.layers.len() {
        return Err(CommandError::failed(format!(
            "the clip has {} layers but only {} lie from layer {} down",
            p.clip.rows.len(),
            tl.layers.len().saturating_sub(p.layer),
            p.layer
        )));
    }
    for (row, frames) in p.clip.rows.iter().enumerate() {
        let Some(l) = tl.layers.get_mut(p.layer.saturating_add(row)) else { continue };
        l.sort_frames();
        if p.overwrite {
            paste_over(l, at, p.clip.width, frames);
        } else {
            paste_inserting(l, at, p.clip.width, frames);
        }
    }
    Ok(json!({ "layer": p.layer, "frame": p.frame, "layers": p.clip.rows.len() }))
}

fn clear_frames(doc: &mut AniDoc, params: &Value) -> Out {
    let p: RangeParams = parse_params(params)?;
    let range = resolve_range(doc, &p)?;
    let width = range.end.saturating_sub(range.from);
    let blank = [Frame::blank(0, width)];
    for l in layers_mut(doc, &range)? {
        l.sort_frames();
        paste_over(l, range.from, width, &blank);
    }
    Ok(json!({ "layers": range.layers, "from": p.from, "to": p.to }))
}

fn reverse_frames(doc: &mut AniDoc, params: &Value) -> Out {
    let p: RangeParams = parse_params(params)?;
    let range = resolve_range(doc, &p)?;
    for l in layers_mut(doc, &range)? {
        l.sort_frames();
        reverse_range(l, range.from, range.end);
    }
    Ok(json!({ "layers": range.layers, "from": p.from, "to": p.to }))
}

fn convert_to_keyframes(doc: &mut AniDoc, params: &Value) -> Out {
    let p: RangeParams = parse_params(params)?;
    let range = resolve_range(doc, &p)?;
    for &layer in &range.layers {
        for frame in range.from..range.end {
            let made = if p.blank {
                edit::insert_blank_keyframe(&mut doc.editor.document, &range.id, layer, frame)
            } else {
                edit::insert_keyframe(&mut doc.editor.document, &range.id, layer, frame)
            };
            match made {
                // Frames that already are keyframes stay as they are.
                Ok(()) | Err(SceneError::AlreadyKeyframe { .. }) => {}
                Err(e) => return Err(failed(e)),
            }
        }
    }
    Ok(json!({ "layers": range.layers, "from": p.from, "to": p.to }))
}

// ---------------------------------------------------------------------------------------------------------------
// The editing position
// ---------------------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnterSymbol {
    name: String,
    #[serde(default)]
    in_place: bool,
    matrix: Option<[f64; 6]>,
}

fn enter_symbol(doc: &mut AniDoc, params: &Value) -> Out {
    let p: EnterSymbol = parse_params(params)?;
    if doc.editor.document.library.find_symbol(&p.name).is_none() {
        return Err(CommandError::failed(format!("the library has no symbol {:?}", p.name)));
    }
    let matrix = if p.in_place {
        let m = p.matrix.unwrap_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        if m.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::invalid_params("matrix must be finite"));
        }
        Some(anim_core::Affine::new(m))
    } else {
        None
    };
    doc.editor.state.enter_symbol(&p.name, matrix);
    Ok(json!({ "timeline": doc.editor.state.timeline, "depth": doc.editor.state.timeline_stack.len() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitTo {
    depth: usize,
}

fn exit_to(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ExitTo = parse_params(params)?;
    doc.editor.state.leave_to_depth(p.depth);
    Ok(json!({ "timeline": doc.editor.state.timeline, "depth": doc.editor.state.timeline_stack.len() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditRange {
    from: Option<u32>,
    to: Option<u32>,
}

fn set_edit_range(doc: &mut AniDoc, params: &Value) -> Out {
    let p: EditRange = parse_params(params)?;
    doc.editor.state.edit_range = match (p.from, p.to) {
        (None, None) => None,
        (Some(a), Some(b)) => {
            let (a, b) = (zero_based(a)?, zero_based(b)?);
            Some((a.min(b), a.max(b)))
        }
        _ => return Err(CommandError::invalid_params("give both `from` and `to`, or neither to switch it off")),
    };
    Ok(json!({ "edit_range": doc.editor.state.edit_range.map(|(a, b)| [a.saturating_add(1), b.saturating_add(1)]) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enabled {
    enabled: bool,
}

fn set_auto_keyframe(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Enabled = parse_params(params)?;
    doc.editor.state.auto_keyframe = p.enabled;
    Ok(json!({ "enabled": p.enabled }))
}

#[cfg(test)]
mod tests {
    use anim_scene::timeline::{EaseCurve, EaseTarget, TweenEase};
    use anim_session::Session;

    use super::*;

    fn session() -> Session<AniDoc> {
        Session::new(AniDoc::default(), super::super::registry())
    }

    fn spans(s: &Session<AniDoc>, layer: usize) -> Vec<(u32, u32)> {
        s.doc().editor.document.scenes[0].layers[layer].frames.iter().map(|f| (f.index, f.duration)).collect()
    }

    fn with_keys(keys: &[(u32, u32)]) -> Session<AniDoc> {
        let mut s = session();
        let layer = &mut s.doc_mut().editor.document.scenes[0].layers[0];
        layer.frames = keys.iter().map(|&(i, d)| Frame::blank(i, d)).collect();
        s
    }

    #[test]
    fn registry_ids_are_unique_and_schemas_valid() {
        let registry = super::super::registry();
        for spec in all() {
            assert!(registry.get(spec.id).is_some(), "{} is registered", spec.id);
            let v: Value = serde_json::from_str(spec.input_schema.unwrap()).unwrap();
            assert_eq!(v["type"], "object", "{}", spec.id);
        }
    }

    #[test]
    fn scenes_move_and_duplicate_and_keep_the_open_scene() {
        let mut s = session();
        s.execute("scene.add", &json!({ "name": "Two" })).unwrap();
        s.execute("scene.add", &json!({ "name": "Three" })).unwrap();
        s.execute("editor.goto", &json!({ "scene": 2 })).unwrap();
        s.execute("scene.move", &json!({ "scene": 2, "to": 0 })).unwrap();
        let names: Vec<_> = s.doc().editor.document.scenes.iter().map(|t| t.name.clone()).collect();
        assert_eq!(names, ["Three", "Scene 1", "Two"]);
        assert_eq!(s.doc().editor.state.timeline, TimelineId::Scene(0), "still editing Three");
        s.execute("scene.duplicate", &json!({ "scene": 0 })).unwrap();
        assert_eq!(s.doc().editor.document.scenes.len(), 4);
        assert_eq!(s.doc().editor.document.scenes[1].name, "Three copy");
        assert_eq!(s.doc().editor.state.timeline, TimelineId::Scene(0));
        s.undo();
        assert_eq!(s.doc().editor.document.scenes.len(), 3);
    }

    #[test]
    fn mask_conversion_masks_the_layer_below_and_releases_it_again() {
        let mut s = session();
        s.execute("layer.add", &json!({ "name": "Top" })).unwrap();
        s.execute("layer.set_kind", &json!({ "layer": 0, "kind": "mask" })).unwrap();
        let tl = &s.doc().editor.document.scenes[0];
        assert_eq!(tl.layers[0].kind, LayerKind::Mask);
        assert_eq!(tl.layers[1].parent_layer_index, Some(0));
        assert_eq!(tl.resolved_kind(1), Some(LayerKind::Masked));
        s.execute("layer.set_kind", &json!({ "layer": 0, "kind": "normal" })).unwrap();
        let tl = &s.doc().editor.document.scenes[0];
        assert_eq!(tl.layers[1].parent_layer_index, None);
        assert_eq!(tl.resolved_kind(1), Some(LayerKind::Normal));
    }

    #[test]
    fn a_layer_with_content_cannot_become_a_folder() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 10, "height": 10, "object_drawing": true })).unwrap();
        let err = s.execute("layer.set_kind", &json!({ "layer": 0, "kind": "folder" })).unwrap_err();
        assert!(err.to_string().contains("content"), "{err}");
        s.execute("layer.add", &json!({})).unwrap();
        s.execute("layer.set_kind", &json!({ "layer": 0, "kind": "folder" })).unwrap();
    }

    #[test]
    fn motion_guide_sits_above_and_guides() {
        let mut s = session();
        s.execute("layer.add_motion_guide", &json!({ "layer": 0 })).unwrap();
        let tl = &s.doc().editor.document.scenes[0];
        assert_eq!(tl.layers[0].kind, LayerKind::Guide);
        assert!(tl.layers[0].name.starts_with("Guide: "));
        assert_eq!(tl.resolved_kind(1), Some(LayerKind::Guided));
        assert!(s.execute("layer.add_motion_guide", &json!({ "layer": 0 })).is_err(), "a guide layer gets no guide");
    }

    #[test]
    fn layer_properties_set_height_colour_and_name() {
        let mut s = session();
        s.execute("layer.set_properties", &json!({ "layer": 0, "name": "Sky", "color": "#ff0000", "height": 2, "locked": true })).unwrap();
        let layer = &s.doc().editor.document.scenes[0].layers[0];
        assert_eq!(layer.name, "Sky");
        assert_eq!(layer.color, Rgba8::opaque(255, 0, 0));
        assert_eq!(layer_height(layer), 2);
        assert!(layer.locked);
        s.execute("layer.set_properties", &json!({ "layer": 0, "height": 1 })).unwrap();
        assert!(s.doc().editor.document.scenes[0].layers[0].extra.attr(HEIGHT_ATTR).is_none());
        assert!(s.execute("layer.set_properties", &json!({ "layer": 0, "height": 5 })).is_err());
    }

    #[test]
    fn copy_then_paste_inserting_pushes_later_frames_right() {
        let mut s = with_keys(&[(0, 3), (3, 3), (6, 4)]);
        let clip = s.execute("timeline.copy_frames", &json!({ "from": 1, "to": 4 })).unwrap()["clip"].clone();
        assert_eq!(clip["width"], 4);
        s.execute("timeline.paste_frames", &json!({ "layer": 0, "frame": 8, "clip": clip })).unwrap();
        // Frame 8 (index 7) splits the last keyframe: [6,7) stays, the clip's [7,10) and [10,11) go in, the rest
        // of the old keyframe follows at [11,14).
        assert_eq!(spans(&s, 0), vec![(0, 3), (3, 3), (6, 1), (7, 3), (10, 1), (11, 3)]);
    }

    #[test]
    fn paste_overwrite_replaces_what_is_there() {
        let mut s = with_keys(&[(0, 10)]);
        let clip = FrameClip { width: 3, rows: vec![vec![Frame::blank(0, 1), Frame::blank(1, 2)]] };
        s.execute("timeline.paste_frames", &json!({ "layer": 0, "frame": 4, "overwrite": true, "clip": clip })).unwrap();
        assert_eq!(spans(&s, 0), vec![(0, 3), (3, 1), (4, 2), (6, 4)]);
    }

    #[test]
    fn cut_removes_the_frames_and_returns_them() {
        let mut s = with_keys(&[(0, 4), (4, 4), (8, 2)]);
        let out = s.execute("timeline.cut_frames", &json!({ "from": 5, "to": 8 })).unwrap();
        assert_eq!(out["clip"]["width"], 4);
        assert_eq!(spans(&s, 0), vec![(0, 4), (4, 2)]);
        s.undo();
        assert_eq!(spans(&s, 0), vec![(0, 4), (4, 4), (8, 2)]);
    }

    #[test]
    fn clear_frames_leaves_one_blank_keyframe_of_the_same_length() {
        let mut s = with_keys(&[(0, 4), (4, 4)]);
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 10, "height": 10, "object_drawing": true })).unwrap();
        s.execute("timeline.clear_frames", &json!({ "from": 3, "to": 6 })).unwrap();
        assert_eq!(spans(&s, 0), vec![(0, 2), (2, 4), (6, 2)]);
        let layer = &s.doc().editor.document.scenes[0].layers[0];
        assert_eq!(layer.frames[1].elements.len(), 0);
    }

    #[test]
    fn reverse_mirrors_keyframes_and_flips_tweens() {
        let mut s = with_keys(&[(0, 2), (2, 3), (5, 1)]);
        {
            let layer = &mut s.doc_mut().editor.document.scenes[0].layers[0];
            layer.frames[0].label = Some("a".into());
            layer.frames[0].tween_type = TweenType::Classic;
            layer.frames[0].acceleration = 40;
            layer.frames[0].eases.push(TweenEase { target: EaseTarget::All, curve: EaseCurve::Named("x".into()) });
        }
        s.execute("timeline.reverse_frames", &json!({ "from": 1, "to": 6 })).unwrap();
        assert_eq!(spans(&s, 0), vec![(0, 1), (1, 3), (4, 2)]);
        let layer = &s.doc().editor.document.scenes[0].layers[0];
        assert_eq!(layer.frames[2].label.as_deref(), Some("a"), "the keyframe keeps its label");
        assert_eq!(layer.frames[1].tween_type, TweenType::Classic, "the tween now starts at the keyframe it used to end at");
        assert_eq!(layer.frames[1].acceleration, -40);
        assert!(!layer.frames[2].is_tweened());
        s.execute("timeline.reverse_frames", &json!({ "from": 1, "to": 6 })).unwrap();
        assert_eq!(spans(&s, 0), vec![(0, 2), (2, 3), (5, 1)]);
        assert_eq!(s.doc().editor.document.scenes[0].layers[0].frames[0].acceleration, 40, "reversing twice restores the ease");
    }

    #[test]
    fn convert_to_keyframes_skips_existing_keyframes() {
        let mut s = with_keys(&[(0, 6)]);
        s.execute("timeline.convert_to_keyframes", &json!({ "from": 2, "to": 4 })).unwrap();
        assert_eq!(spans(&s, 0), vec![(0, 1), (1, 1), (2, 1), (3, 3)]);
        s.execute("timeline.convert_to_keyframes", &json!({ "from": 1, "to": 3, "blank": true })).unwrap();
        assert_eq!(spans(&s, 0).len(), 4);
    }

    #[test]
    fn the_editing_position_commands_move_between_levels() {
        let mut s = session();
        s.doc_mut().editor.document.add_symbol("box", anim_scene::SymbolKind::MovieClip).unwrap();
        s.execute("edit.enter_symbol", &json!({ "name": "box" })).unwrap();
        assert_eq!(s.doc().editor.state.timeline, TimelineId::Symbol("box".into()));
        assert!(s.doc().editor.state.in_place.is_none());
        s.execute("edit.enter_symbol", &json!({ "name": "box", "in_place": true, "matrix": [1, 0, 0, 1, 40, 30] })).unwrap();
        assert!(s.doc().editor.state.in_place.is_some());
        assert_eq!(s.doc().editor.state.timeline_stack.len(), 2);
        s.execute("edit.exit_to", &json!({ "depth": 1 })).unwrap();
        assert!(s.doc().editor.state.in_place.is_none());
        s.execute("edit.exit_to", &json!({ "depth": 0 })).unwrap();
        assert_eq!(s.doc().editor.state.timeline, TimelineId::Scene(0));
        assert!(s.execute("edit.enter_symbol", &json!({ "name": "nope" })).is_err());
    }

    #[test]
    fn commands_follow_the_timeline_being_edited() {
        let mut s = session();
        s.doc_mut().editor.document.add_symbol("box", anim_scene::SymbolKind::MovieClip).unwrap();
        s.execute("edit.enter_symbol", &json!({ "name": "box" })).unwrap();
        s.execute("timeline.insert_frames", &json!({ "layer": 0, "frame": 1, "count": 4 })).unwrap();
        let symbol = s.doc().editor.document.library.find_symbol("box").unwrap();
        assert_eq!(symbol.timeline.frame_count(), 5);
        assert_eq!(s.doc().editor.document.scenes[0].frame_count(), 1, "the scene did not change");
    }

    #[test]
    fn edit_range_and_auto_keyframe_are_set_by_commands() {
        let mut s = session();
        s.execute("editor.set_edit_range", &json!({ "from": 3, "to": 1 })).unwrap();
        assert_eq!(s.doc().editor.state.edit_range, Some((0, 2)));
        assert!(s.execute("editor.set_edit_range", &json!({ "from": 3 })).is_err());
        s.execute("editor.set_edit_range", &json!({})).unwrap();
        assert_eq!(s.doc().editor.state.edit_range, None);
        s.execute("editor.set_auto_keyframe", &json!({ "enabled": true })).unwrap();
        assert!(s.doc().editor.state.auto_keyframe);
    }
}
