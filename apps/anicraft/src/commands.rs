//! Editor commands that need the Properties panel's model: the property setter.
//!
//! Everything else (Modify and Insert menu items, instance properties, tween options, import) lives in
//! `anicraft_cli::mcp::editing`, so the headless server has it too. The registry here is that one plus
//! `selection.set_property`, so every menu item of the editor is also an MCP tool and an undo step.

use anicraft_cli::mcp::AniDoc;
use anim_session::{CommandError, CommandRegistry, CommandSpec, parse_params};
use serde::Deserialize;
use serde_json::{Value, json};

pub use anicraft_cli::mcp::editing::paste_elements;

type Out = Result<Value, CommandError>;

/// The registry of the editor: undo/redo, the document commands, the tool commands, the editing commands and these.
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
        CommandSpec::new("selection.set_property", "Set Property", set_property)
            .describe("Sets one property of the single selected object (or of the document when nothing is selected, with ids doc.width, doc.height, doc.fps, doc.background). The ids are those the Properties panel shows: x, y, w, h, rotation, name, visible, effect, alpha, blend, fill, stroke, stroke_width, text, font, size, text_color, direction, align, ... Call tools_state to see what is selected.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["property","value"],"properties":{"property":{"type":"string"},"value":{}}}"#),
    ]
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

#[cfg(test)]
mod tests {
    use anicraft_tools::{ElementRef, Selection};
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
        assert!(r.get("instance.add_filter").is_some(), "editing commands are present");
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
