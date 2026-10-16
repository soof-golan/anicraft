//! Editor commands that the library crates do not provide: Modify and Insert menu items and the property setter.
//!
//! They are registered next to the document commands of `anicraft-cli` and the tool commands of `anicraft-tools`, so
//! every menu item of the editor is also an MCP tool and an undo step.

use anicraft_cli::mcp::AniDoc;
use anicraft_tools::{ElementRef, Selection};
use anim_core::{Affine, Point};
use anim_scene::edit::{self, Registration, SymbolSpec};
use anim_scene::{Document, Element, Group, SymbolKind, TimelineId};
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use serde::Deserialize;
use serde_json::{Value, json};

type Out = Result<Value, CommandError>;

/// The registry of the editor: undo/redo, the document commands, the tool commands and these.
///
/// # Panics
/// Never in practice: a test registers every id.
#[must_use]
pub fn registry() -> CommandRegistry<AniDoc> {
    let mut registry = anicraft_cli::mcp::registry();
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(all()).expect("editor command ids are unique");
    registry
}

/// The commands of this module.
#[must_use]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
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
        CommandSpec::new("selection.set_property", "Set Property", set_property)
            .describe("Sets one property of the single selected object (or of the document when nothing is selected, with ids doc.width, doc.height, doc.fps, doc.background). The ids are those the Properties panel shows: x, y, w, h, rotation, name, visible, effect, alpha, blend, fill, stroke, stroke_width, text, font, size, text_color, direction, align, ... Call tools_state to see what is selected.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["property","value"],"properties":{"property":{"type":"string"},"value":{}}}"#),
        CommandSpec::new("edit.copy", "Copy", copy_selection)
            .in_menu(&["Edit"])
            .with_shortcut("Cmd+C")
            .enabled_when(has_selection)
            .transient()
            .describe("Returns the selected objects as JSON (the editor keeps them on its clipboard).")
            .with_schema(NO_PARAMS),
    ]
}

const NO_PARAMS: &str = r#"{"type":"object","additionalProperties":false,"properties":{}}"#;

fn has_selection(doc: &AniDoc) -> bool {
    !doc.editor.state.selection.is_empty()
}

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::failed(e.to_string())
}

fn kind_of(name: Option<&str>) -> Result<SymbolKind, CommandError> {
    match name {
        None | Some("graphic") => Ok(SymbolKind::Graphic),
        Some("movie_clip") => Ok(SymbolKind::MovieClip),
        Some("button") => Ok(SymbolKind::Button),
        Some(other) => Err(CommandError::invalid_params(format!("unknown symbol kind {other:?}"))),
    }
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

fn timeline(doc: &AniDoc) -> TimelineId {
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
    let id = timeline(doc);
    let at = edit::convert_to_symbol(&mut doc.editor.document, &id, layer, keyframe, &indices, &spec).map_err(failed)?;
    select(doc, layer, keyframe, [at]);
    Ok(json!({ "symbol": p.name, "instance_index": at }))
}

fn group(doc: &mut AniDoc, _params: &Value) -> Out {
    let (layer, keyframe, indices) = one_keyframe(doc)?;
    let id = timeline(doc);
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
    let id = timeline(doc);
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
                let content = library.find_symbol(&inst.library_item_name).and_then(|s| s.timeline.layers.iter().rev().find_map(|l| l.keyframe_at(inst.first_frame).map(|k| k.elements.clone()))).unwrap_or_default();
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
    let id = timeline(doc);
    let frame = edit::keyframe_mut(&mut doc.editor.document, &id, layer, keyframe).map_err(failed)?;
    let count = frame.elements.len();
    let mut order: Vec<usize> = (0..count).collect();
    let selected = |i: &usize| indices.contains(i);
    match p.mode.as_str() {
        "front" => {
            let (mut chosen, mut rest): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
            for i in order {
                if selected(&i) { chosen.push(i) } else { rest.push(i) }
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
    let id = timeline(doc);
    let refs = doc.editor.state.selection.items.clone();
    let rects = crate::view::selection_rects(&doc.editor.document, &id, &refs);
    let bounds = rects.into_iter().reduce(|a, b| a.union(b)).ok_or_else(|| CommandError::failed("the selection has no bounds"))?;
    let c = bounds.center();
    let mirror = match p.axis.as_str() {
        "horizontal" => Affine::translate((c.x, c.y)) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate((-c.x, -c.y)),
        "vertical" => Affine::translate((c.x, c.y)) * Affine::scale_non_uniform(1.0, -1.0) * Affine::translate((-c.x, -c.y)),
        other => return Err(CommandError::invalid_params(format!("unknown axis {other:?}"))),
    };
    for r in &refs {
        let Some(el) = crate::props::element_mut(&mut doc.editor.document, &id, r) else { continue };
        if let Some(m) = el.matrix() {
            // Elements inside groups are flipped in stage space, so conjugate by the parent transform.
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetPropertyParams {
    property: String,
    value: Value,
}

fn set_property(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SetPropertyParams = parse_params(params)?;
    let value = crate::props::from_json(doc, &p.property, &p.value).map_err(CommandError::invalid_params)?;
    crate::props::apply(doc, &p.property, &value).map_err(CommandError::failed)?;
    Ok(json!({ "property": p.property }))
}

fn copy_selection(doc: &mut AniDoc, _params: &Value) -> Out {
    let id = timeline(doc);
    let elements: Vec<&Element> = doc.editor.state.selection.items.iter().filter_map(|r| anicraft_tools::resolve_ref(&doc.editor.document, &id, r)).map(|r| r.element).collect();
    Ok(json!({ "count": elements.len(), "elements": serde_json::to_value(&elements).map_err(failed)? }))
}

/// Elements on the clipboard, pasted at the playhead of the current layer.
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

#[cfg(test)]
mod tests {
    use anim_session::Session;

    use super::*;

    fn session() -> Session<AniDoc> {
        Session::new(AniDoc::default(), registry())
    }

    #[test]
    fn registry_has_unique_ids_and_valid_schemas() {
        let r = registry();
        assert!(r.get("modify.group").is_some());
        assert!(r.get("draw.rectangle").is_some(), "tool commands are present");
        assert!(r.get("layer.add").is_some(), "document commands are present");
        for spec in all() {
            serde_json::from_str::<Value>(spec.input_schema.unwrap()).unwrap();
        }
    }

    #[test]
    fn draw_group_flip_and_convert_are_undoable() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 10, "y": 10, "width": 100, "height": 60, "fill": "#ff0000", "object_drawing": true })).unwrap();
        s.execute("draw.oval", &json!({ "x": 150, "y": 10, "width": 50, "height": 50, "fill": "#0000ff", "object_drawing": true })).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        assert_eq!(s.doc().editor.state.selection.items.len(), 2);
        s.execute("modify.group", &json!({})).unwrap();
        let frame = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap();
        assert_eq!(frame.elements.len(), 1);
        s.execute("modify.flip", &json!({ "axis": "horizontal" })).unwrap();
        s.execute("modify.convert_to_symbol", &json!({ "name": "pair", "kind": "movie_clip" })).unwrap();
        assert!(s.doc().editor.document.library.find_symbol("pair").is_some());
        s.undo();
        assert!(s.doc().editor.document.library.find_symbol("pair").is_none());
        s.execute("modify.ungroup", &json!({})).unwrap();
        let frame = s.doc().editor.document.scenes[0].layers[0].keyframe_at(0).unwrap();
        assert_eq!(frame.elements.len(), 2);
    }

    #[test]
    fn arranging_reorders_and_keeps_the_selection() {
        let mut s = session();
        s.execute("draw.rectangle", &json!({ "x": 0, "y": 0, "width": 10, "height": 10, "object_drawing": true })).unwrap();
        s.execute("draw.rectangle", &json!({ "x": 20, "y": 0, "width": 10, "height": 10, "object_drawing": true })).unwrap();
        s.doc_mut().editor.state.selection = Selection::one(ElementRef::top_level(0, 0, 0));
        s.execute("modify.arrange", &json!({ "mode": "front" })).unwrap();
        assert_eq!(s.doc().editor.state.selection.items, vec![ElementRef::top_level(0, 0, 1)]);
        assert!(s.execute("modify.arrange", &json!({ "mode": "sideways" })).is_err());
    }

    #[test]
    fn properties_go_through_a_command() {
        let mut s = session();
        s.execute("selection.set_property", &json!({ "property": "doc.width", "value": 640 })).unwrap();
        assert!((s.doc().editor.document.width - 640.0).abs() < f64::EPSILON);
        assert!(s.execute("selection.set_property", &json!({ "property": "doc.width", "value": "wide" })).is_err());
        assert!(s.execute("selection.set_property", &json!({ "property": "x", "value": 1 })).is_err());
    }

    #[test]
    fn commands_that_need_a_selection_are_disabled_without_one() {
        let s = session();
        assert!(!s.is_enabled("modify.group"));
        assert!(s.is_enabled("insert.symbol"));
    }
}
