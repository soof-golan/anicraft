//! Object-level editing commands.
//!
//! The Modify and Insert menus, instance properties (colour effect, filters, blend mode, graphic playback), tween
//! options, frame sounds and scripts, layer state and library import. They work on the **selection** (what `draw_*`, `select_*` and the editor's mouse tools produced) or, when `layer`,
//! `frame` and `index` are all given, on that one object of the timeline being edited (`edit_enter` switches it).
//! `frame` is 1-based like everywhere else; `layer` and `index` are 0-based. Every command is one undo step and a
//! failure changes nothing.

use std::path::Path;

use anicraft_tools::{ElementRef, Selection};
use anim_core::{Affine, BlendMode, ColorTransform, Point, Rgba8, num};
use anim_scene::edit::{self, Registration, SymbolSpec};
use anim_scene::timeline::{SoundLoop, SoundSync, TweenRotate};
use anim_scene::{ColorEffect, Document, Element, FrameSound, Group, InstanceFilter, LoopMode, SymbolKind, TextAlign, TextDirection, TimelineId};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use serde::Deserialize;
use serde_json::{Value, json};

use super::commands::{timeline, zero_based};
use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

/// The `layer`/`frame`/`index` properties every object command accepts to address one object explicitly.
macro_rules! target_props {
    () => {
        r#""layer":{"type":"integer","minimum":0,"description":"with frame and index: address this object instead of the selection"},"frame":{"type":"integer","minimum":1,"description":"1-based"},"index":{"type":"integer","minimum":0,"description":"position in the keyframe, as stage_elements lists it"}"#
    };
}

const NO_PARAMS: &str = r#"{"type":"object","additionalProperties":false,"properties":{}}"#;

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::failed(e.to_string())
}

/// All commands of this module.
#[must_use]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
    let mut specs = menu_commands();
    specs.extend(instance_commands());
    specs.extend(timeline_commands());
    specs
}

/// Registers [`all`] into `registry`.
///
/// # Panics
/// Never in practice: a test registers every id.
pub fn register(registry: &mut CommandRegistry<AniDoc>) {
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(all()).expect("editing command ids are unique");
}

fn menu_commands() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("modify.convert_to_symbol", "Convert to Symbol...", convert_to_symbol)
            .in_menu(&["Modify"])
            .with_shortcut("F8")
            .enabled_when(has_selection)
            .describe("Moves the selected objects of one keyframe into a new library symbol and puts an instance in their place. `kind` is graphic, movie_clip or button; `registration` is center, top_left or origin.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["name"],"properties":{"name":{"type":"string","minLength":1},"kind":{"enum":["graphic","movie_clip","button"]},"registration":{"enum":["center","top_left","origin"]}}}"#),
        CommandSpec::new("modify.group", "Group", group)
            .in_menu(&["Modify"])
            .with_shortcut("Cmd+G")
            .enabled_when(has_selection)
            .describe("Groups the selected objects.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.ungroup", "Ungroup", ungroup)
            .in_menu(&["Modify"])
            .with_shortcut("Cmd+Shift+G")
            .enabled_when(has_selection)
            .describe("Replaces each selected group by its members.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.break_apart", "Break Apart", break_apart)
            .in_menu(&["Modify"])
            .with_shortcut("Cmd+B")
            .enabled_when(has_selection)
            .describe("Replaces each selected symbol instance by the content of its first frame, and each group by its members.")
            .with_schema(NO_PARAMS),
        CommandSpec::new("modify.arrange", "Arrange", arrange)
            .in_menu(&["Modify"])
            .enabled_when(has_selection)
            .describe("Changes the stacking order of the selected objects within their keyframe: front, forward, backward or back.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["mode"],"properties":{"mode":{"enum":["front","forward","backward","back"]}}}"#),
        CommandSpec::new("modify.flip", "Flip", flip)
            .in_menu(&["Modify"])
            .enabled_when(has_selection)
            .describe("Mirrors the selection about the centre of its bounds, horizontally or vertically.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["axis"],"properties":{"axis":{"enum":["horizontal","vertical"]}}}"#),
        CommandSpec::new("insert.symbol", "New Symbol...", new_symbol)
            .in_menu(&["Insert"])
            .with_shortcut("Cmd+F8")
            .describe("Adds an empty symbol to the library. `kind` is graphic (default), movie_clip or button.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["name"],"properties":{"name":{"type":"string","minLength":1},"kind":{"enum":["graphic","movie_clip","button"]}}}"#),
        CommandSpec::new("edit.copy", "Copy", copy_selection)
            .in_menu(&["Edit"])
            .with_shortcut("Cmd+C")
            .enabled_when(has_selection)
            .transient()
            .describe("Returns the selected objects as JSON (the editor keeps them on its clipboard).")
            .with_schema(NO_PARAMS),
        CommandSpec::new("library.import", "Import to Library...", import)
            .in_menu(&["File", "Import"])
            .describe("Imports an image into the library: PNG, JPEG, GIF, BMP and WebP as they are; PSD, AI, PDF and SVG flattened to a PNG at their own size. `name` overrides the library name. With `place` the bitmap is also put, centred, on the current layer and frame of the timeline being edited. The path must lie inside a read root.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"type":"string","description":"file to import"},"name":{"type":"string"},"place":{"type":"boolean","default":false}}}"#),
    ]
}

fn instance_commands() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("instance.set_color_effect", "Color Effect", set_color_effect)
            .describe("Sets the Color effect of the selected symbol instances (Properties > Color Effect). `effect`: none; brightness (`value` -100..100 %); tint (`color` #rrggbb, `amount` 0..100 %); alpha (`value` 0..100 %); advanced (`mul` and `add`: four numbers each for r,g,b,a; `add` in -1..1).")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["effect"],"properties":{"#, target_props!(), r##","effect":{"enum":["none","brightness","tint","alpha","advanced"]},"value":{"type":"number"},"color":{"type":"string","description":"#rrggbb"},"amount":{"type":"number","minimum":0,"maximum":100},"mul":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4},"add":{"type":"array","items":{"type":"number"},"minItems":4,"maxItems":4}}}"##)),
        CommandSpec::new("instance.add_filter", "Add Filter", add_filter)
            .describe("Adds a filter to the selected symbol instances. `kind`: blur (`x`, `y` px, `quality` 1..3); drop_shadow (`color`, `angle` degrees, `distance`, `blur_x`, `blur_y`, `strength` where 1 is 100 %, `inner`, `knockout`, `hide_object`); glow (`color`, `blur_x`, `blur_y`, `strength`, `inner`, `knockout`); color_matrix (`matrix`: 20 numbers, row-major 4x5).")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["kind"],"properties":{"#, target_props!(), r##","kind":{"enum":["blur","drop_shadow","glow","color_matrix"]},"x":{"type":"number","minimum":0,"maximum":255},"y":{"type":"number","minimum":0,"maximum":255},"quality":{"type":"integer","minimum":1,"maximum":3},"color":{"type":"string","description":"#rrggbb or #rrggbbaa"},"angle":{"type":"number"},"distance":{"type":"number"},"blur_x":{"type":"number","minimum":0,"maximum":255},"blur_y":{"type":"number","minimum":0,"maximum":255},"strength":{"type":"number","minimum":0,"maximum":255},"inner":{"type":"boolean"},"knockout":{"type":"boolean"},"hide_object":{"type":"boolean"},"matrix":{"type":"array","items":{"type":"number"},"minItems":20,"maxItems":20}}}"##)),
        CommandSpec::new("instance.clear_filters", "Clear Filters", clear_filters)
            .describe("Removes every filter from the selected symbol instances.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"properties":{"#, target_props!(), "}}")),
        CommandSpec::new("instance.set_options", "Instance Options", set_options)
            .describe("Sets instance properties on the selected symbol instances: `blend_mode` (normal, layer, multiply, screen, overlay, darken, lighten, color_dodge, color_burn, hard_light, soft_light, difference, exclusion, add, subtract, invert, alpha, erase, ...), `name` (instance name, empty clears), `visible`, `cache_as_bitmap`, and for graphics `loop_mode` (loop, play_once, single_frame) with `first_frame` (1-based).")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"properties":{"#, target_props!(), r#","blend_mode":{"type":"string"},"name":{"type":"string"},"visible":{"type":"boolean"},"cache_as_bitmap":{"type":"boolean"},"loop_mode":{"enum":["loop","play_once","single_frame"]},"first_frame":{"type":"integer","minimum":1}}}"#)),
        CommandSpec::new("text.set_style", "Text Style", set_text_style)
            .describe("Restyles the selected text fields (Properties > Character and Paragraph): `text` replaces the content, `face`, `size`, `color`, `bold`, `italic`, `letter_spacing`, `direction` (auto picks right-to-left from Hebrew or Arabic letters, or force ltr or rtl) and `align` (left, right, center, justify).")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"properties":{"#, target_props!(), r#","text":{"type":"string"},"face":{"type":"string"},"size":{"type":"number","minimum":1},"color":{"type":"string"},"bold":{"type":"boolean"},"italic":{"type":"boolean"},"letter_spacing":{"type":"number"},"direction":{"enum":["auto","ltr","rtl"]},"align":{"enum":["left","right","center","justify"]}}}"#)),
    ]
}

fn timeline_commands()-> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("timeline.set_tween_options", "Tween Options", set_tween_options)
            .describe("Sets the options of the classic tween that starts at the keyframe: `rotate` (auto, none, clockwise, counter_clockwise) with `rotate_times` full extra turns, `scale`, `snap`, `orient_to_path`, `sync`.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer","frame"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based"}"#, r#","rotate":{"enum":["auto","none","clockwise","counter_clockwise"]},"rotate_times":{"type":"number","minimum":0,"maximum":100},"scale":{"type":"boolean"},"snap":{"type":"boolean"},"orient_to_path":{"type":"boolean"},"sync":{"type":"boolean"}}}"#)),
        CommandSpec::new("timeline.set_sound", "Set Frame Sound", set_sound)
            .describe("Attaches a library sound to the keyframe at `frame` (or clears it when `sound` is omitted). `sync`: event (default), start, stop, stream; `repeat` plays it that many times in total; `forever` loops it.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer","frame"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based"}"#, r#","sound":{"type":"string","description":"library name of a sound"},"sync":{"enum":["event","start","stop","stream"]},"repeat":{"type":"integer","minimum":1,"maximum":10000},"forever":{"type":"boolean"}}}"#)),
        CommandSpec::new("timeline.set_script", "Set Frame Script", set_script)
            .describe("Sets the ActionScript of the keyframe at `frame` (kept verbatim; AniCraft does not run it). An empty `script` clears it.")
            .with_schema(concat!(r#"{"type":"object","additionalProperties":false,"required":["layer","frame","script"],"properties":{"#, r#""scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based"}"#, r#","script":{"type":"string"}}}"#)),
        CommandSpec::new("timeline.move_keyframe", "Move Keyframe", move_keyframe)
            .describe("Moves the keyframe at `from` to the empty frame `to` on a layer.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","from","to"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"from":{"type":"integer","minimum":1},"to":{"type":"integer","minimum":1}}}"#),
        CommandSpec::new("layer.duplicate", "Duplicate Layer", duplicate_layer)
            .describe("Duplicates a layer with all its frames; the copy goes above it.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0}}}"#),
        CommandSpec::new("layer.set_state", "Layer State", set_layer_state)
            .describe("Shows, hides, locks or outlines a layer, and (re)parents it to a folder, mask or guide layer above it (`parent`, or `clear_parent`).")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"visible":{"type":"boolean"},"locked":{"type":"boolean"},"outline":{"type":"boolean"},"parent":{"type":"integer","minimum":0},"clear_parent":{"type":"boolean"}}}"#),
    ]
}

const fn has_selection(doc: &AniDoc) -> bool {
    !doc.editor.state.selection.is_empty()
}

fn kind_of(name: Option<&str>) -> Result<SymbolKind, CommandError> {
    match name {
        None | Some("graphic") => Ok(SymbolKind::Graphic),
        Some("movie_clip") => Ok(SymbolKind::MovieClip),
        Some("button") => Ok(SymbolKind::Button),
        Some(other) => Err(CommandError::invalid_params(format!("unknown symbol kind {other:?}"))),
    }
}

/// Mutable access to the element a reference points at.
pub fn element_mut<'a>(doc: &'a mut Document, id: &TimelineId, r: &ElementRef) -> Option<&'a mut Element> {
    let layer = doc.timeline_mut(id)?.layers.get_mut(r.layer)?;
    let frame = layer.frames.iter_mut().find(|f| f.index == r.keyframe)?;
    let mut elements = &mut frame.elements;
    let (last, ancestors) = r.path.split_last()?;
    for &i in ancestors {
        let Some(Element::Group(g)) = elements.get_mut(i) else { return None };
        elements = &mut g.members;
    }
    elements.get_mut(*last)
}

/// Stage-space bounds of each referenced object.
#[must_use]
pub fn selection_rects(doc: &Document, id: &TimelineId, refs: &[ElementRef]) -> Vec<anim_core::Rect> {
    let bounds = anicraft_tools::LibraryBounds::new(&doc.library);
    refs.iter()
        .filter_map(|r| {
            let resolved = anicraft_tools::resolve_ref(doc, id, r)?;
            let local = anicraft_tools::hit::bounds_in_parent(&bounds, resolved.element, 0)?;
            anicraft_tools::hit::transform_rect(resolved.parent_to_stage, local)
        })
        .collect()
}

/// The selection as (layer, keyframe, sorted top-level indices), or an error when it spans several keyframes.
fn one_keyframe(doc: &AniDoc) -> Result<(usize, u32, Vec<usize>), CommandError> {
    let items = &doc.editor.state.selection.items;
    let first = items.first().ok_or_else(|| CommandError::failed("nothing is selected"))?;
    let mut indices = Vec::new();
    for r in items {
        if r.layer != first.layer || r.keyframe != first.keyframe {
            return Err(CommandError::failed("the selection spans several layers or keyframes; select objects of one keyframe"));
        }
        let [index] = r.path.as_slice() else { return Err(CommandError::failed("select top-level objects (not members of a group)")) };
        indices.push(*index);
    }
    indices.sort_unstable();
    indices.dedup();
    Ok((first.layer, first.keyframe, indices))
}

fn select(doc: &mut AniDoc, layer: usize, keyframe: u32, indices: impl IntoIterator<Item = usize>) {
    doc.editor.state.selection = Selection { items: indices.into_iter().map(|i| ElementRef::top_level(layer, keyframe, i)).collect(), scope: None };
}

fn editing(doc: &AniDoc) -> TimelineId {
    doc.editor.state.timeline.clone()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConvertParams {
    name: String,
    kind: Option<String>,
    registration: Option<String>,
}

fn convert_to_symbol(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ConvertParams = parse_params(params)?;
    let (layer, keyframe, indices) = one_keyframe(doc)?;
    let registration = match p.registration.as_deref() {
        None | Some("center") => Registration::Center,
        Some("top_left") => Registration::TopLeft,
        Some("origin") => Registration::Origin,
        Some(other) => return Err(CommandError::invalid_params(format!("unknown registration {other:?}"))),
    };
    let spec = SymbolSpec { name: &p.name, kind: kind_of(p.kind.as_deref())?, registration };
    let id = editing(doc);
    let at = edit::convert_to_symbol(&mut doc.editor.document, &id, layer, keyframe, &indices, &spec).map_err(failed)?;
    select(doc, layer, keyframe, [at]);
    Ok(json!({ "symbol": p.name, "instance_index": at }))
}

fn group(doc: &mut AniDoc, _params: &Value) -> Out {
    let (layer, keyframe, indices) = one_keyframe(doc)?;
    let id = editing(doc);
    let members = edit::remove_elements(&mut doc.editor.document, &id, layer, keyframe, &indices).map_err(failed)?;
    let count = members.len();
    let at = indices.first().copied().unwrap_or(0);
    let frame = edit::keyframe_mut(&mut doc.editor.document, &id, layer, keyframe).map_err(failed)?;
    let at = at.min(frame.elements.len());
    frame.elements.insert(at, Element::Group(Group::new(members)));
    select(doc, layer, keyframe, [at]);
    Ok(json!({ "grouped": count, "index": at }))
}

fn ungroup(doc: &mut AniDoc, _params: &Value) -> Out {
    expand(doc, false)
}

fn break_apart(doc: &mut AniDoc, _params: &Value) -> Out {
    expand(doc, true)
}

/// Replaces groups (and, with `symbols`, instances) by their content.
fn expand(doc: &mut AniDoc, symbols: bool) -> Out {
    let (layer, keyframe, indices) = one_keyframe(doc)?;
    let id = editing(doc);
    let library = doc.editor.document.library.clone();
    let frame = edit::keyframe_mut(&mut doc.editor.document, &id, layer, keyframe).map_err(failed)?;
    let mut produced = Vec::new();
    let mut touched = 0_usize;
    let mut result = Vec::with_capacity(frame.elements.len());
    for (i, element) in std::mem::take(&mut frame.elements).into_iter().enumerate() {
        if !indices.contains(&i) {
            result.push(element);
            continue;
        }
        match element {
            Element::Group(g) => {
                touched = touched.saturating_add(1);
                for mut member in g.members {
                    if let Some(m) = member.matrix() {
                        member.set_matrix(g.matrix * m);
                    }
                    produced.push(result.len());
                    result.push(member);
                }
            }
            Element::SymbolInstance(inst) if symbols => {
                let content = library
                    .find_symbol(&inst.library_item_name)
                    .and_then(|s| s.timeline.layers.iter().rev().find_map(|l| l.keyframe_at(inst.first_frame).map(|k| k.elements.clone())))
                    .unwrap_or_default();
                touched = touched.saturating_add(1);
                for mut member in content {
                    if let Some(m) = member.matrix() {
                        member.set_matrix(inst.matrix * m);
                    }
                    produced.push(result.len());
                    result.push(member);
                }
            }
            other => {
                produced.push(result.len());
                result.push(other);
            }
        }
    }
    frame.elements = result;
    if touched == 0 {
        return Err(CommandError::failed(if symbols { "nothing selected can be broken apart" } else { "no group is selected" }));
    }
    select(doc, layer, keyframe, produced);
    Ok(json!({ "expanded": touched }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ArrangeParams {
    mode: String,
}

fn arrange(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ArrangeParams = parse_params(params)?;
    let (layer, keyframe, indices) = one_keyframe(doc)?;
    let id = editing(doc);
    let frame = edit::keyframe_mut(&mut doc.editor.document, &id, layer, keyframe).map_err(failed)?;
    let count = frame.elements.len();
    let mut order: Vec<usize> = (0..count).collect();
    let selected = |i: &usize| indices.contains(i);
    match p.mode.as_str() {
        "front" => {
            let (mut chosen, mut rest): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
            for i in order {
                if selected(&i) {
                    chosen.push(i);
                } else {
                    rest.push(i);
                }
            }
            rest.extend(chosen);
            order = rest;
        }
        "back" => {
            let (mut chosen, rest): (Vec<usize>, Vec<usize>) = order.into_iter().partition(selected);
            chosen.extend(rest);
            order = chosen;
        }
        "forward" => {
            for k in (0..count.saturating_sub(1)).rev() {
                let next = k.saturating_add(1);
                if let (Some(a), Some(b)) = (order.get(k).copied(), order.get(next).copied())
                    && selected(&a)
                    && !selected(&b)
                {
                    order.swap(k, next);
                }
            }
        }
        "backward" => {
            for k in 1..count {
                let prev = k.saturating_sub(1);
                if let (Some(a), Some(b)) = (order.get(prev).copied(), order.get(k).copied())
                    && selected(&b)
                    && !selected(&a)
                {
                    order.swap(prev, k);
                }
            }
        }
        other => return Err(CommandError::invalid_params(format!("unknown mode {other:?}"))),
    }
    let mut taken: Vec<Option<Element>> = std::mem::take(&mut frame.elements).into_iter().map(Some).collect();
    frame.elements = order.iter().filter_map(|i| taken.get_mut(*i).and_then(Option::take)).collect();
    let new_positions: Vec<usize> = order.iter().enumerate().filter(|(_, old)| indices.contains(old)).map(|(new, _)| new).collect();
    select(doc, layer, keyframe, new_positions);
    Ok(json!({ "mode": p.mode }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FlipParams {
    axis: String,
}

fn flip(doc: &mut AniDoc, params: &Value) -> Out {
    let p: FlipParams = parse_params(params)?;
    let id = editing(doc);
    let refs = doc.editor.state.selection.items.clone();
    let rects = selection_rects(&doc.editor.document, &id, &refs);
    let bounds = rects.into_iter().reduce(|a, b| a.union(b)).ok_or_else(|| CommandError::failed("the selection has no bounds"))?;
    let c = bounds.center();
    let mirror = match p.axis.as_str() {
        "horizontal" => Affine::translate((c.x, c.y)) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate((-c.x, -c.y)),
        "vertical" => Affine::translate((c.x, c.y)) * Affine::scale_non_uniform(1.0, -1.0) * Affine::translate((-c.x, -c.y)),
        other => return Err(CommandError::invalid_params(format!("unknown axis {other:?}"))),
    };
    for r in &refs {
        let Some(el) = element_mut(&mut doc.editor.document, &id, r) else { continue };
        if let Some(m) = el.matrix() {
            el.set_matrix(mirror * m);
        }
    }
    Ok(json!({ "flipped": refs.len(), "axis": p.axis }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewSymbolParams {
    name: String,
    kind: Option<String>,
}

fn new_symbol(doc: &mut AniDoc, params: &Value) -> Out {
    let p: NewSymbolParams = parse_params(params)?;
    doc.editor.document.add_symbol(&p.name, kind_of(p.kind.as_deref())?).map_err(failed)?;
    Ok(json!({ "symbol": p.name }))
}

fn copy_selection(doc: &mut AniDoc, _params: &Value) -> Out {
    let id = editing(doc);
    let elements: Vec<&Element> = doc.editor.state.selection.items.iter().filter_map(|r| anicraft_tools::resolve_ref(&doc.editor.document, &id, r)).map(|r| r.element).collect();
    Ok(json!({ "count": elements.len(), "elements": serde_json::to_value(&elements).map_err(failed)? }))
}

/// Elements on the clipboard, pasted at `frame` of `layer`, moved by `offset`.
///
/// # Errors
/// When the layer or keyframe does not exist.
pub fn paste_elements(doc: &mut Document, id: &TimelineId, layer: usize, frame: u32, elements: Vec<Element>, offset: Point) -> Result<Vec<usize>, anim_scene::SceneError> {
    let mut at = Vec::new();
    for mut element in elements {
        if let Some(m) = element.matrix() {
            element.set_matrix(Affine::translate(offset.to_vec2()) * m);
        }
        at.push(edit::add_element(doc, id, layer, frame, element)?);
    }
    Ok(at)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImportParams {
    path: String,
    name: Option<String>,
    #[serde(default)]
    place: bool,
}

fn import(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ImportParams = parse_params(params)?;
    let imported = crate::import::import_artwork_as(doc, Path::new(&p.path), p.name.as_deref(), p.place).map_err(CommandError::failed)?;
    Ok(json!({ "name": imported.name, "width": imported.size[0], "height": imported.size[1], "placed": p.place }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Instance commands
// ---------------------------------------------------------------------------------------------------------------------

/// Which objects an instance command acts on.
fn targets(doc: &AniDoc, layer: Option<usize>, frame: Option<u32>, index: Option<usize>) -> Result<Vec<ElementRef>, CommandError> {
    match (layer, frame, index) {
        (None, None, None) => {
            let items = doc.editor.state.selection.items.clone();
            if items.is_empty() {
                Err(CommandError::failed("nothing is selected: select objects first, or pass layer, frame and index"))
            } else {
                Ok(items)
            }
        }
        (Some(layer), Some(frame), Some(index)) => {
            let id = editing(doc);
            let frame0 = zero_based(frame)?;
            let keyframe = doc
                .document()
                .timeline(&id)
                .and_then(|t| t.layers.get(layer))
                .and_then(|l| l.keyframe_at(frame0))
                .ok_or_else(|| CommandError::failed(format!("layer {layer} has no keyframe covering frame {frame}")))?;
            Ok(vec![ElementRef::top_level(layer, keyframe.index, index)])
        }
        _ => Err(CommandError::invalid_params("pass layer, frame and index together, or none of them to use the selection")),
    }
}

/// Runs `f` on every targeted symbol instance; fails when there is none.
fn on_instances(
    doc: &mut AniDoc,
    (layer, frame, index): (Option<usize>, Option<u32>, Option<usize>),
    mut f: impl FnMut(&mut anim_scene::SymbolInstance) -> Result<(), CommandError>,
) -> Result<usize, CommandError> {
    let refs = targets(doc, layer, frame, index)?;
    let id = editing(doc);
    let mut touched = 0_usize;
    for r in &refs {
        if let Some(Element::SymbolInstance(instance)) = element_mut(&mut doc.editor.document, &id, r) {
            f(instance)?;
            touched = touched.saturating_add(1);
        }
    }
    if touched == 0 {
        return Err(CommandError::failed("no symbol instance among the targets (graphics, movie clips and buttons have these properties; shapes, text and bitmaps do not)"));
    }
    Ok(touched)
}

/// Runs `f` on every targeted text field; fails when there is none.
fn on_texts(
    doc: &mut AniDoc,
    (layer, frame, index): (Option<usize>, Option<u32>, Option<usize>),
    mut f: impl FnMut(&mut anim_scene::TextElement) -> Result<(), CommandError>,
) -> Result<usize, CommandError> {
    let refs = targets(doc, layer, frame, index)?;
    let id = editing(doc);
    let mut touched = 0_usize;
    for r in &refs {
        if let Some(Element::Text(text)) = element_mut(&mut doc.editor.document, &id, r) {
            f(text)?;
            touched = touched.saturating_add(1);
        }
    }
    if touched == 0 {
        return Err(CommandError::failed("no text field among the targets (draw one with draw_text)"));
    }
    Ok(touched)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextStyleParams {
    layer: Option<usize>,
    frame: Option<u32>,
    index: Option<usize>,
    text: Option<String>,
    face: Option<String>,
    size: Option<f64>,
    color: Option<String>,
    bold: Option<bool>,
    italic: Option<bool>,
    direction: Option<String>,
    align: Option<String>,
    letter_spacing: Option<f64>,
}

fn set_text_style(doc: &mut AniDoc, params: &Value) -> Out {
    let p: TextStyleParams = parse_params(params)?;
    let color = p.color.as_deref().map(colour).transpose()?;
    let direction = match p.direction.as_deref() {
        None => None,
        Some("auto") => Some(TextDirection::Auto),
        Some("ltr") => Some(TextDirection::LeftToRight),
        Some("rtl") => Some(TextDirection::RightToLeft),
        Some(other) => return Err(CommandError::invalid_params(format!("unknown direction {other:?}; use auto, ltr or rtl"))),
    };
    let align = match p.align.as_deref() {
        None => None,
        Some("left") => Some(TextAlign::Left),
        Some("right") => Some(TextAlign::Right),
        Some("center") => Some(TextAlign::Center),
        Some("justify") => Some(TextAlign::Justify),
        Some(other) => return Err(CommandError::invalid_params(format!("unknown alignment {other:?}"))),
    };
    for v in [p.size, p.letter_spacing].into_iter().flatten() {
        finite(&[v], "size and letter_spacing")?;
    }
    if p.size.is_some_and(|s| s < 1.0) {
        return Err(CommandError::invalid_params("size must be at least 1"));
    }
    let touched = on_texts(doc, (p.layer, p.frame, p.index), |t| {
        if let Some(new) = &p.text {
            let first = t.text_runs.first().cloned();
            if let Some(mut run) = first {
                run.characters.clone_from(new);
                t.text_runs = vec![run];
            }
        }
        for run in &mut t.text_runs {
            if let Some(v) = &p.face {
                run.face.clone_from(v);
            }
            if let Some(v) = p.size {
                run.size = num::f64_to_f32(v);
            }
            if let Some(v) = color {
                run.fill_color = v;
            }
            if let Some(v) = p.bold {
                run.bold = v;
            }
            if let Some(v) = p.italic {
                run.italic = v;
            }
            if let Some(v) = direction {
                run.direction = v;
            }
            if let Some(v) = align {
                run.alignment = v;
            }
            if let Some(v) = p.letter_spacing {
                run.letter_spacing = num::f64_to_f32(v);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "text_fields": touched }))
}

fn colour(text: &str) -> Result<Rgba8, CommandError> {
    super::commands::parse_color(text)
}

fn finite(values: &[f64], what: &str) -> Result<(), CommandError> {
    if values.iter().all(|v| v.is_finite()) { Ok(()) } else { Err(CommandError::invalid_params(format!("{what} must be finite numbers"))) }
}

fn percent(value: f64, lo: f64, hi: f64, what: &str) -> Result<f32, CommandError> {
    if !value.is_finite() || value < lo || value > hi {
        return Err(CommandError::invalid_params(format!("{what} must be between {lo} and {hi} percent")));
    }
    Ok(num::f64_to_f32(value / 100.0))
}

fn four(values: &[f64], what: &str) -> Result<[f32; 4], CommandError> {
    finite(values, what)?;
    match values {
        [a, b, c, d] => Ok([num::f64_to_f32(*a), num::f64_to_f32(*b), num::f64_to_f32(*c), num::f64_to_f32(*d)]),
        _ => Err(CommandError::invalid_params(format!("{what} needs four numbers"))),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ColorParams {
    layer: Option<usize>,
    frame: Option<u32>,
    index: Option<usize>,
    effect: String,
    value: Option<f64>,
    color: Option<String>,
    amount: Option<f64>,
    mul: Option<Vec<f64>>,
    add: Option<Vec<f64>>,
}

fn set_color_effect(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ColorParams = parse_params(params)?;
    let need = |v: Option<f64>, what: &str| v.ok_or_else(|| CommandError::invalid_params(format!("{} needs `{what}`", p.effect)));
    let effect = match p.effect.as_str() {
        "none" => ColorEffect::None,
        "brightness" => ColorEffect::Brightness(percent(need(p.value, "value")?, -100.0, 100.0, "brightness")?),
        "alpha" => ColorEffect::Alpha(percent(need(p.value, "value")?, 0.0, 100.0, "alpha")?),
        "tint" => {
            let color = colour(p.color.as_deref().ok_or_else(|| CommandError::invalid_params("tint needs `color`"))?)?;
            ColorEffect::Tint { color, amount: percent(p.amount.unwrap_or(100.0), 0.0, 100.0, "tint amount")? }
        }
        "advanced" => {
            let mul = four(p.mul.as_deref().unwrap_or(&[1.0, 1.0, 1.0, 1.0]), "mul")?;
            let add = four(p.add.as_deref().unwrap_or(&[0.0, 0.0, 0.0, 0.0]), "add")?;
            ColorEffect::Advanced(ColorTransform { mul, add })
        }
        other => return Err(CommandError::invalid_params(format!("unknown colour effect {other:?}"))),
    };
    let touched = on_instances(doc, (p.layer, p.frame, p.index), |i| {
        i.set_color_effect(effect);
        Ok(())
    })?;
    Ok(json!({ "effect": p.effect, "instances": touched }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterParams {
    layer: Option<usize>,
    frame: Option<u32>,
    index: Option<usize>,
    kind: String,
    x: Option<f64>,
    y: Option<f64>,
    quality: Option<u8>,
    color: Option<String>,
    angle: Option<f64>,
    distance: Option<f64>,
    blur_x: Option<f64>,
    blur_y: Option<f64>,
    strength: Option<f64>,
    #[serde(default)]
    inner: bool,
    #[serde(default)]
    knockout: bool,
    #[serde(default)]
    hide_object: bool,
    matrix: Option<Vec<f64>>,
}

fn add_filter(doc: &mut AniDoc, params: &Value) -> Out {
    let p: FilterParams = parse_params(params)?;
    let blur = |v: Option<f64>, what: &str| -> Result<f64, CommandError> {
        let v = v.unwrap_or(5.0);
        if v.is_finite() && (0.0..=255.0).contains(&v) { Ok(v) } else { Err(CommandError::invalid_params(format!("{what} must be between 0 and 255"))) }
    };
    let strength = p.strength.unwrap_or(1.0);
    finite(&[strength, p.angle.unwrap_or(0.0), p.distance.unwrap_or(0.0)], "angle, distance and strength")?;
    let shadow_color = |default: &str| colour(p.color.as_deref().unwrap_or(default));
    let filter = match p.kind.as_str() {
        "blur" => anim_display::Filter::Blur { x: blur(p.x, "x")?, y: blur(p.y, "y")?, quality: p.quality.unwrap_or(1).clamp(1, 3) },
        "drop_shadow" => anim_display::Filter::DropShadow {
            color: shadow_color("#000000")?,
            angle_deg: p.angle.unwrap_or(45.0),
            distance: p.distance.unwrap_or(5.0),
            blur_x: blur(p.blur_x, "blur_x")?,
            blur_y: blur(p.blur_y, "blur_y")?,
            strength,
            inner: p.inner,
            knockout: p.knockout,
            hide_object: p.hide_object,
        },
        "glow" => anim_display::Filter::Glow { color: shadow_color("#ff0000")?, blur_x: blur(p.blur_x, "blur_x")?, blur_y: blur(p.blur_y, "blur_y")?, strength, inner: p.inner, knockout: p.knockout },
        "color_matrix" => {
            let values = p.matrix.as_deref().ok_or_else(|| CommandError::invalid_params("color_matrix needs `matrix` (20 numbers)"))?;
            finite(values, "matrix")?;
            let mut matrix = [0.0_f32; 20];
            if values.len() != matrix.len() {
                return Err(CommandError::invalid_params("`matrix` needs exactly 20 numbers"));
            }
            for (slot, value) in matrix.iter_mut().zip(values) {
                *slot = num::f64_to_f32(*value);
            }
            anim_display::Filter::ColorMatrix(matrix)
        }
        other => return Err(CommandError::invalid_params(format!("unknown filter {other:?}"))),
    };
    let touched = on_instances(doc, (p.layer, p.frame, p.index), |i| {
        i.filters.push(InstanceFilter::mapped(filter.clone()));
        Ok(())
    })?;
    Ok(json!({ "filter": p.kind, "instances": touched }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetOnly {
    layer: Option<usize>,
    frame: Option<u32>,
    index: Option<usize>,
}

fn clear_filters(doc: &mut AniDoc, params: &Value) -> Out {
    let p: TargetOnly = parse_params(params)?;
    let touched = on_instances(doc, (p.layer, p.frame, p.index), |i| {
        i.filters.clear();
        Ok(())
    })?;
    Ok(json!({ "instances": touched }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionsParams {
    layer: Option<usize>,
    frame: Option<u32>,
    index: Option<usize>,
    blend_mode: Option<String>,
    name: Option<String>,
    visible: Option<bool>,
    cache_as_bitmap: Option<bool>,
    loop_mode: Option<String>,
    first_frame: Option<u32>,
}

/// `color_dodge` -> `ColorDodge`, the spelling of the model's variants.
fn blend_of(name: &str) -> Result<BlendMode, CommandError> {
    let pascal: String = name
        .split(['_', ' ', '-'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect::<String>()).unwrap_or_default()
        })
        .collect();
    serde_json::from_value(Value::String(pascal)).map_err(|_| CommandError::invalid_params(format!("unknown blend mode {name:?}")))
}

fn set_options(doc: &mut AniDoc, params: &Value) -> Out {
    let p: OptionsParams = parse_params(params)?;
    let blend = p.blend_mode.as_deref().map(blend_of).transpose()?;
    let loop_mode = match p.loop_mode.as_deref() {
        None => None,
        Some("loop") => Some(LoopMode::Loop),
        Some("play_once") => Some(LoopMode::PlayOnce),
        Some("single_frame") => Some(LoopMode::SingleFrame),
        Some(other) => return Err(CommandError::invalid_params(format!("unknown loop mode {other:?}"))),
    };
    let first = p.first_frame.map(zero_based).transpose()?;
    if blend.is_none() && p.name.is_none() && p.visible.is_none() && p.cache_as_bitmap.is_none() && loop_mode.is_none() && first.is_none() {
        return Err(CommandError::invalid_params("nothing to set: pass blend_mode, name, visible, cache_as_bitmap, loop_mode or first_frame"));
    }
    let touched = on_instances(doc, (p.layer, p.frame, p.index), |i| {
        if let Some(b) = blend {
            i.blend_mode = b;
        }
        if let Some(name) = &p.name {
            i.name = Some(name.clone()).filter(|n| !n.is_empty());
        }
        if let Some(v) = p.visible {
            i.visible = v;
        }
        if let Some(v) = p.cache_as_bitmap {
            i.cache_as_bitmap = v;
        }
        if let Some(m) = loop_mode {
            i.loop_mode = Some(m);
        }
        if let Some(f) = first {
            i.first_frame = f;
        }
        Ok(())
    })?;
    Ok(json!({ "instances": touched }))
}

// ---------------------------------------------------------------------------------------------------------------------
// Timeline commands
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TweenParams {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    rotate: Option<String>,
    rotate_times: Option<f64>,
    scale: Option<bool>,
    snap: Option<bool>,
    orient_to_path: Option<bool>,
    sync: Option<bool>,
}

fn set_tween_options(doc: &mut AniDoc, params: &Value) -> Out {
    let p: TweenParams = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let rotate = match p.rotate.as_deref() {
        None => None,
        Some("auto") => Some(TweenRotate::Auto),
        Some("none") => Some(TweenRotate::None),
        Some("clockwise") => Some(TweenRotate::Clockwise),
        Some("counter_clockwise") => Some(TweenRotate::CounterClockwise),
        Some(other) => return Err(CommandError::invalid_params(format!("unknown rotate mode {other:?}"))),
    };
    if let Some(times) = p.rotate_times {
        finite(&[times], "rotate_times")?;
    }
    let frame = edit::keyframe_mut(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?).map_err(failed)?;
    if frame.tween_type == anim_scene::timeline::TweenType::None {
        return Err(CommandError::failed("there is no tween on this keyframe; create one with timeline_create_classic_tween first"));
    }
    if let Some(r) = rotate {
        frame.motion_tween_rotate = r;
    }
    if let Some(t) = p.rotate_times {
        frame.motion_tween_rotate_times = Some(t);
    }
    if let Some(v) = p.scale {
        frame.motion_tween_scale = v;
    }
    if let Some(v) = p.snap {
        frame.motion_tween_snap = v;
    }
    if let Some(v) = p.orient_to_path {
        frame.motion_tween_orient_to_path = v;
    }
    if let Some(v) = p.sync {
        frame.motion_tween_sync = v;
    }
    Ok(json!({ "layer": p.layer, "frame": p.frame }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SoundParams {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    sound: Option<String>,
    sync: Option<String>,
    repeat: Option<u32>,
    #[serde(default)]
    forever: bool,
}

fn set_sound(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SoundParams = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let sound = match &p.sound {
        None => None,
        Some(name) => {
            if doc.editor.document.library.find_sound(name).is_none() {
                return Err(CommandError::failed(format!("the library has no sound {name:?}")));
            }
            let mut s = FrameSound::new(name.clone());
            s.sync = match p.sync.as_deref() {
                None | Some("event") => SoundSync::Event,
                Some("start") => SoundSync::Start,
                Some("stop") => SoundSync::Stop,
                Some("stream") => SoundSync::Stream,
                Some(other) => return Err(CommandError::invalid_params(format!("unknown sync {other:?}"))),
            };
            s.loop_mode = if p.forever {
                SoundLoop::Forever
            } else {
                match p.repeat {
                    None | Some(0 | 1) => SoundLoop::Once,
                    Some(n) => SoundLoop::Repeat(n.min(10_000)),
                }
            };
            Some(s)
        }
    };
    let frame = edit::keyframe_mut(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?).map_err(failed)?;
    frame.sound = sound;
    Ok(json!({ "layer": p.layer, "frame": p.frame, "sound": p.sound }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptParams {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    script: String,
}

fn set_script(doc: &mut AniDoc, params: &Value) -> Out {
    let p: ScriptParams = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let frame = edit::keyframe_mut(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?).map_err(failed)?;
    frame.actionscript = Some(p.script.clone()).filter(|s| !s.is_empty());
    Ok(json!({ "layer": p.layer, "frame": p.frame, "bytes": p.script.len() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveKeyframe {
    scene: Option<usize>,
    layer: usize,
    from: u32,
    to: u32,
}

fn move_keyframe(doc: &mut AniDoc, params: &Value) -> Out {
    let p: MoveKeyframe = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    edit::move_keyframe(&mut doc.editor.document, &id, p.layer, zero_based(p.from)?, zero_based(p.to)?).map_err(failed)?;
    Ok(json!({ "layer": p.layer, "from": p.from, "to": p.to }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerOnly {
    scene: Option<usize>,
    layer: usize,
}

fn duplicate_layer(doc: &mut AniDoc, params: &Value) -> Out {
    let p: LayerOnly = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let index = edit::duplicate_layer(&mut doc.editor.document, &id, p.layer).map_err(failed)?;
    Ok(json!({ "layer": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerState {
    scene: Option<usize>,
    layer: usize,
    visible: Option<bool>,
    locked: Option<bool>,
    outline: Option<bool>,
    parent: Option<usize>,
    #[serde(default)]
    clear_parent: bool,
}

fn set_layer_state(doc: &mut AniDoc, params: &Value) -> Out {
    let p: LayerState = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    if p.parent.is_some() && p.clear_parent {
        return Err(CommandError::invalid_params("pass `parent` or `clear_parent`, not both"));
    }
    if p.parent.is_some() || p.clear_parent {
        edit::set_layer_parent(&mut doc.editor.document, &id, p.layer, p.parent).map_err(failed)?;
    }
    let layer = doc
        .editor
        .document
        .timeline_mut(&id)
        .and_then(|t| t.layers.get_mut(p.layer))
        .ok_or_else(|| CommandError::failed(format!("no layer {}", p.layer)))?;
    if let Some(v) = p.visible {
        layer.visible = v;
    }
    if let Some(v) = p.locked {
        layer.locked = v;
    }
    if let Some(v) = p.outline {
        layer.outline = v;
    }
    Ok(json!({ "layer": p.layer, "visible": layer.visible, "locked": layer.locked, "outline": layer.outline, "parent": layer.parent_layer_index }))
}

#[cfg(test)]
mod tests {
    use anim_session::Session;

    use super::*;

    fn session() -> Session<AniDoc> {
        let mut registry = CommandRegistry::with_history_commands();
        register(&mut registry);
        registry.register_all(anicraft_tools::mcp_commands::<AniDoc>()).unwrap();
        Session::new(AniDoc::default(), registry)
    }

    fn instance(s: &Session<AniDoc>) -> anim_scene::SymbolInstance {
        let found = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap().elements.iter().find_map(|e| match e {
            Element::SymbolInstance(i) => Some(i.clone()),
            _ => None,
        });
        *found.unwrap()
    }

    fn with_instance(s: &mut Session<AniDoc>) {
        s.execute("draw.rectangle", &json!({ "x": 10, "y": 10, "width": 40, "height": 40, "fill": "#ff0000", "object_drawing": true })).unwrap();
        s.execute("modify.convert_to_symbol", &json!({ "name": "box", "kind": "movie_clip" })).unwrap();
    }

    #[test]
    fn every_schema_is_valid_json_describing_an_object() {
        for spec in all() {
            let schema: Value = serde_json::from_str(spec.input_schema.unwrap()).unwrap_or_else(|e| panic!("{}: {e}", spec.id));
            assert_eq!(schema["type"], "object", "{}", spec.id);
        }
    }

    #[test]
    fn ids_are_unique() {
        let _ = session();
    }

    #[test]
    fn colour_effects_and_filters_apply_to_the_selection_and_undo() {
        let mut s = session();
        with_instance(&mut s);
        s.execute("instance.set_color_effect", &json!({ "effect": "alpha", "value": 40 })).unwrap();
        s.execute("instance.add_filter", &json!({ "kind": "blur", "x": 6, "y": 6 })).unwrap();
        s.execute("instance.add_filter", &json!({ "kind": "drop_shadow", "color": "#00000080", "distance": 8 })).unwrap();
        s.execute("instance.set_options", &json!({ "blend_mode": "color_dodge", "name": "hero" })).unwrap();
        let i = instance(&s);
        assert!(matches!(i.color_effect, ColorEffect::Alpha(a) if (a - 0.4).abs() < 1e-6));
        assert_eq!(i.filters.len(), 2);
        assert_eq!(i.blend_mode, BlendMode::ColorDodge);
        assert_eq!(i.name.as_deref(), Some("hero"));
        s.execute("instance.clear_filters", &json!({})).unwrap();
        assert_eq!(instance(&s).filters.len(), 0);
        s.undo();
        assert_eq!(instance(&s).filters.len(), 2);
    }

    #[test]
    fn bad_instance_parameters_fail_and_change_nothing() {
        let mut s = session();
        with_instance(&mut s);
        for bad in [
            json!({ "effect": "alpha" }),
            json!({ "effect": "alpha", "value": 400 }),
            json!({ "effect": "tint", "color": "red" }),
            json!({ "effect": "sparkle" }),
            json!({ "effect": "advanced", "mul": [1, 2] }),
        ] {
            assert!(s.execute("instance.set_color_effect", &bad).is_err(), "{bad}");
        }
        assert!(s.execute("instance.add_filter", &json!({ "kind": "blur", "x": 1e9 })).is_err());
        assert!(s.execute("instance.add_filter", &json!({ "kind": "color_matrix", "matrix": [1, 2, 3] })).is_err());
        assert!(s.execute("instance.set_options", &json!({})).is_err());
        assert!(s.execute("instance.set_options", &json!({ "blend_mode": "sparkle" })).is_err());
        assert!(s.execute("instance.set_options", &json!({ "layer": 0 })).is_err(), "layer without frame and index");
        s.execute("select.none", &json!({})).unwrap();
        assert!(s.execute("instance.clear_filters", &json!({})).is_err(), "nothing selected");
    }

    #[test]
    fn an_object_can_be_addressed_without_a_selection() {
        let mut s = session();
        with_instance(&mut s);
        s.execute("select.none", &json!({})).unwrap();
        s.execute("instance.set_color_effect", &json!({ "layer": 0, "frame": 1, "index": 0, "effect": "tint", "color": "#3366ff", "amount": 50 })).unwrap();
        let k = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap();
        let Some(Element::SymbolInstance(i)) = k.elements.first() else { panic!("no instance") };
        assert!(matches!(i.color_effect, ColorEffect::Tint { .. }));
        assert!(s.execute("instance.set_color_effect", &json!({ "layer": 0, "frame": 1, "index": 5, "effect": "none" })).is_err());
    }

    #[test]
    fn tween_options_need_a_tween() {
        let mut s = session();
        assert!(s.execute("timeline.set_tween_options", &json!({ "layer": 0, "frame": 1, "rotate": "clockwise" })).is_err());
    }

    #[test]
    fn layer_state_and_duplicates() {
        let mut s = session();
        s.execute("layer.set_state", &json!({ "layer": 0, "visible": false, "locked": true })).unwrap();
        let l = &s.doc().editor.document.scenes[0].layers[0];
        assert!(!l.visible && l.locked);
        assert!(s.execute("layer.set_state", &json!({ "layer": 0, "parent": 0, "clear_parent": true })).is_err());
        assert!(s.execute("layer.set_state", &json!({ "layer": 9, "visible": true })).is_err());
    }

    #[test]
    fn blend_names_are_forgiving() {
        assert_eq!(blend_of("hard_light").unwrap(), BlendMode::HardLight);
        assert_eq!(blend_of("Multiply").unwrap(), BlendMode::Multiply);
        assert_eq!(blend_of("soft light").unwrap(), BlendMode::SoftLight);
        assert!(blend_of("").is_err());
    }

    #[test]
    fn importing_needs_a_readable_image() {
        let mut s = session();
        assert!(s.execute("library.import", &json!({ "path": "/definitely/missing.png" })).is_err());
    }
}
