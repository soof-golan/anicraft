//! The editing commands offered to MCP clients (and usable by any other front end).
//!
//! Conventions, repeated in every schema: scenes and layers are 0-based indices exactly as `inspect_document` lists
//! them (layer 0 is the top layer); frames are 1-based like Animate's timeline; `scene` defaults to the scene the
//! document has open. Each command is one undo step and runs on a copy, so a failure changes nothing.

use anim_core::{Affine, Rgba8};
use anim_scene::edit;
use anim_scene::timeline::{LabelType, LayerKind};
use anim_scene::{SceneError, TimelineId};
use anim_session::{CommandError, CommandSpec, parse_params};
use serde::Deserialize;
use serde_json::{Value, json};

use super::doc::AniDoc;

type Out = Result<Value, CommandError>;

fn failed(e: &SceneError) -> CommandError {
    CommandError::failed(e.to_string())
}

/// All commands, in menu order.
#[must_use]
pub fn all() -> Vec<CommandSpec<AniDoc>> {
    vec![
        CommandSpec::new("document.set_stage", "Set Stage", set_stage)
            .in_menu(&["Modify"])
            .describe("Changes the stage size in pixels, the frame rate or the background colour. Omitted fields stay as they are.")
            .with_schema(r##"{"type":"object","additionalProperties":false,"properties":{"width":{"type":"number","exclusiveMinimum":0},"height":{"type":"number","exclusiveMinimum":0},"frame_rate":{"type":"number","exclusiveMinimum":0,"maximum":1000},"background":{"type":"string","description":"#rrggbb or #rrggbbaa"}}}"##),
        CommandSpec::new("scene.add", "Add Scene", scene_add)
            .in_menu(&["Insert"])
            .describe("Adds an empty scene (one layer with a blank keyframe) at position `at` (default: the end).")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["name"],"properties":{"name":{"type":"string","minLength":1},"at":{"type":"integer","minimum":0}}}"#),
        CommandSpec::new("scene.remove", "Remove Scene", scene_remove)
            .describe("Removes a scene. The last scene cannot be removed.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["scene"],"properties":{"scene":{"type":"integer","minimum":0}}}"#),
        CommandSpec::new("scene.rename", "Rename Scene", scene_rename)
            .describe("Renames a scene.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["scene","name"],"properties":{"scene":{"type":"integer","minimum":0},"name":{"type":"string","minLength":1}}}"#),
        CommandSpec::new("layer.add", "Add Layer", layer_add)
            .in_menu(&["Insert"])
            .describe("Adds a layer at index `at` (default: on top). `kind` is normal, folder, guide or mask.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"properties":{"scene":{"type":"integer","minimum":0},"at":{"type":"integer","minimum":0},"name":{"type":"string"},"kind":{"enum":["normal","folder","guide","mask"]}}}"#),
        CommandSpec::new("layer.remove", "Remove Layer", layer_remove)
            .describe("Deletes a layer with its frames.")
            .with_schema(LAYER_ONLY),
        CommandSpec::new("layer.rename", "Rename Layer", layer_rename)
            .describe("Renames a layer.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","name"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"name":{"type":"string","minLength":1}}}"#),
        CommandSpec::new("layer.move", "Move Layer", layer_move)
            .describe("Moves a layer to a new index.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["from","to"],"properties":{"scene":{"type":"integer","minimum":0},"from":{"type":"integer","minimum":0},"to":{"type":"integer","minimum":0}}}"#),
        CommandSpec::new("timeline.insert_keyframe", "Insert Keyframe", insert_keyframe)
            .describe("Makes `frame` a keyframe on a layer, copying the content that was showing there.")
            .with_schema(LAYER_FRAME),
        CommandSpec::new("timeline.insert_blank_keyframe", "Insert Blank Keyframe", insert_blank_keyframe)
            .describe("Makes `frame` an empty keyframe on a layer.")
            .with_schema(LAYER_FRAME),
        CommandSpec::new("timeline.clear_keyframe", "Clear Keyframe", clear_keyframe)
            .describe("Empties the keyframe at `frame`.")
            .with_schema(LAYER_FRAME),
        CommandSpec::new("timeline.insert_frames", "Insert Frames", insert_frames)
            .describe("Inserts `count` frames at `frame`, pushing later keyframes later.")
            .with_schema(LAYER_FRAME_COUNT),
        CommandSpec::new("timeline.remove_frames", "Remove Frames", remove_frames)
            .describe("Removes `count` frames starting at `frame`.")
            .with_schema(LAYER_FRAME_COUNT),
        CommandSpec::new("timeline.create_classic_tween", "Create Classic Tween", create_classic_tween)
            .describe("Turns the keyframe at `frame` into a classic tween towards the next keyframe.")
            .with_schema(LAYER_FRAME),
        CommandSpec::new("timeline.create_shape_tween", "Create Shape Tween", create_shape_tween)
            .describe("Turns the keyframe at `frame` into a shape tween towards the next keyframe.")
            .with_schema(LAYER_FRAME),
        CommandSpec::new("timeline.remove_tween", "Remove Tween", remove_tween)
            .describe("Removes the tween that starts at `frame`.")
            .with_schema(LAYER_FRAME),
        CommandSpec::new("timeline.set_ease", "Set Ease", set_ease)
            .describe("Sets the classic ease of the tween at `frame`: -100 (ease in) to 100 (ease out), 0 is linear.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","frame","acceleration"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1},"acceleration":{"type":"integer","minimum":-100,"maximum":100}}}"#),
        CommandSpec::new("timeline.set_label", "Set Frame Label", set_label)
            .describe("Names the keyframe at `frame`; without `label` the label is removed.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","frame"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1},"label":{"type":"string"}}}"#),
        CommandSpec::new("stage.add_symbol_instance", "Place Symbol", add_instance)
            .describe("Places an instance of a library symbol on the keyframe at `frame`, with its origin at (x, y) stage pixels.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","frame","symbol"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1},"symbol":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"}}}"#),
        CommandSpec::new("stage.remove_elements", "Remove Elements", remove_elements)
            .describe("Removes elements (by index in the keyframe) from the keyframe at `frame`.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["layer","frame","indices"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1},"indices":{"type":"array","minItems":1,"items":{"type":"integer","minimum":0}}}}"#),
        CommandSpec::new("library.rename_item", "Rename Library Item", rename_item)
            .describe("Renames a library item and every instance that refers to it.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["name","new_name"],"properties":{"name":{"type":"string"},"new_name":{"type":"string","minLength":1}}}"#),
        CommandSpec::new("library.remove_item", "Remove Library Item", remove_item)
            .describe("Deletes a library item. Instances of it become empty placeholders in Animate, so check `query_references` first.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["name"],"properties":{"name":{"type":"string"}}}"#),
        CommandSpec::new("library.duplicate_symbol", "Duplicate Symbol", duplicate_symbol)
            .describe("Copies a symbol under a new name.")
            .with_schema(r#"{"type":"object","additionalProperties":false,"required":["name","new_name"],"properties":{"name":{"type":"string"},"new_name":{"type":"string","minLength":1}}}"#),
    ]
}

const LAYER_ONLY: &str = r#"{"type":"object","additionalProperties":false,"required":["layer"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0}}}"#;
const LAYER_FRAME: &str = r#"{"type":"object","additionalProperties":false,"required":["layer","frame"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based"}}}"#;
const LAYER_FRAME_COUNT: &str = r#"{"type":"object","additionalProperties":false,"required":["layer","frame","count"],"properties":{"scene":{"type":"integer","minimum":0},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1},"count":{"type":"integer","minimum":1}}}"#;

/// Resolves the `scene` parameter against the document.
fn timeline(doc: &AniDoc, scene: Option<usize>) -> Result<TimelineId, CommandError> {
    let last = doc.editor.document.scenes.len().saturating_sub(1);
    let index = scene.unwrap_or_else(|| doc.editor.document.current_timeline.min(last));
    if index < doc.editor.document.scenes.len() {
        Ok(TimelineId::Scene(index))
    } else {
        Err(CommandError::failed(format!("no scene {index}; the document has {}", doc.editor.document.scenes.len())))
    }
}

/// 1-based frame to 0-based.
fn zero_based(frame: u32) -> Result<u32, CommandError> {
    frame.checked_sub(1).ok_or_else(|| CommandError::invalid_params("frames start at 1"))
}

fn parse_color(text: &str) -> Result<Rgba8, CommandError> {
    let digits = text.strip_prefix('#').unwrap_or(text);
    let byte = |i: usize| digits.get(i..i.saturating_add(2)).and_then(|s| u8::from_str_radix(s, 16).ok());
    match digits.len() {
        6 => Ok(Rgba8 {
            r: byte(0).ok_or_else(|| bad_color(text))?,
            g: byte(2).ok_or_else(|| bad_color(text))?,
            b: byte(4).ok_or_else(|| bad_color(text))?,
            a: 255,
        }),
        8 => Ok(Rgba8 {
            r: byte(0).ok_or_else(|| bad_color(text))?,
            g: byte(2).ok_or_else(|| bad_color(text))?,
            b: byte(4).ok_or_else(|| bad_color(text))?,
            a: byte(6).ok_or_else(|| bad_color(text))?,
        }),
        _ => Err(bad_color(text)),
    }
}

fn bad_color(text: &str) -> CommandError {
    CommandError::invalid_params(format!("{text:?} is not a colour like #rrggbb"))
}

/// Wraps a command's result value, so every command ends the same way.
#[expect(clippy::unnecessary_wraps, clippy::missing_const_for_fn, reason = "keeps the command bodies uniform")]
fn ok(value: Value) -> Out {
    Ok(value)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StageParams {
    width: Option<f64>,
    height: Option<f64>,
    frame_rate: Option<f64>,
    background: Option<String>,
}

fn set_stage(doc: &mut AniDoc, params: &Value) -> Out {
    let p: StageParams = parse_params(params)?;
    let positive = |v: f64, what: &str| {
        if v.is_finite() && v > 0.0 && v <= 16384.0 { Ok(v) } else { Err(CommandError::invalid_params(format!("{what} {v} is out of range"))) }
    };
    if let Some(w) = p.width {
        doc.editor.document.width = positive(w, "width")?;
    }
    if let Some(h) = p.height {
        doc.editor.document.height = positive(h, "height")?;
    }
    if let Some(f) = p.frame_rate {
        if !(f.is_finite() && f > 0.0 && f <= 1000.0) {
            return Err(CommandError::invalid_params(format!("frame_rate {f} is out of range")));
        }
        doc.editor.document.frame_rate = f;
    }
    if let Some(c) = p.background {
        doc.editor.document.background_color = parse_color(&c)?;
    }
    ok(json!({ "width": doc.editor.document.width, "height": doc.editor.document.height, "frame_rate": doc.editor.document.frame_rate }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneAdd {
    name: String,
    at: Option<usize>,
}

fn scene_add(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SceneAdd = parse_params(params)?;
    let at = p.at.unwrap_or(doc.editor.document.scenes.len());
    let index = edit::add_scene(&mut doc.editor.document, &p.name, at).map_err(|e| failed(&e))?;
    ok(json!({ "scene": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneOnly {
    scene: usize,
}

fn scene_remove(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SceneOnly = parse_params(params)?;
    let removed = edit::remove_scene(&mut doc.editor.document, p.scene).map_err(|e| failed(&e))?;
    ok(json!({ "removed": removed.name }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneRename {
    scene: usize,
    name: String,
}

fn scene_rename(doc: &mut AniDoc, params: &Value) -> Out {
    let p: SceneRename = parse_params(params)?;
    if p.name.trim().is_empty() {
        return Err(CommandError::invalid_params("a scene needs a name"));
    }
    let scene = doc.editor.document.scenes.get_mut(p.scene).ok_or_else(|| CommandError::failed(format!("no scene {}", p.scene)))?;
    p.name.clone_into(&mut scene.name);
    ok(json!({ "scene": p.scene, "name": p.name }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerAdd {
    scene: Option<usize>,
    at: Option<usize>,
    name: Option<String>,
    kind: Option<String>,
}

fn layer_add(doc: &mut AniDoc, params: &Value) -> Out {
    let p: LayerAdd = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let kind = match p.kind.as_deref().unwrap_or("normal") {
        "normal" => LayerKind::Normal,
        "folder" => LayerKind::Folder,
        "guide" => LayerKind::Guide,
        "mask" => LayerKind::Mask,
        other => return Err(CommandError::invalid_params(format!("unknown layer kind {other:?}"))),
    };
    let at = p.at.unwrap_or(0);
    let index = edit::add_layer(&mut doc.editor.document, &id, at, p.name.as_deref(), kind).map_err(|e| failed(&e))?;
    ok(json!({ "layer": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerRef {
    scene: Option<usize>,
    layer: usize,
}

fn layer_remove(doc: &mut AniDoc, params: &Value) -> Out {
    let p: LayerRef = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let remaining = edit::remove_layer(&mut doc.editor.document, &id, p.layer).map_err(|e| failed(&e))?;
    ok(json!({ "layers_left": remaining }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerRename {
    scene: Option<usize>,
    layer: usize,
    name: String,
}

fn layer_rename(doc: &mut AniDoc, params: &Value) -> Out {
    let p: LayerRename = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    edit::rename_layer(&mut doc.editor.document, &id, p.layer, &p.name).map_err(|e| failed(&e))?;
    ok(json!({ "layer": p.layer, "name": p.name }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LayerMove {
    scene: Option<usize>,
    from: usize,
    to: usize,
}

fn layer_move(doc: &mut AniDoc, params: &Value) -> Out {
    let p: LayerMove = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let index = edit::move_layer(&mut doc.editor.document, &id, p.from, p.to).map_err(|e| failed(&e))?;
    ok(json!({ "layer": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct At {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
}

/// Parses `At` params and runs `f` with the timeline, layer and 0-based frame.
fn at(doc: &mut AniDoc, params: &Value, f: impl FnOnce(&mut anim_scene::Document, &TimelineId, usize, u32) -> Result<(), SceneError>) -> Out {
    let p: At = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    f(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?).map_err(|e| failed(&e))?;
    ok(json!({ "layer": p.layer, "frame": p.frame }))
}

fn insert_keyframe(doc: &mut AniDoc, params: &Value) -> Out {
    at(doc, params, edit::insert_keyframe)
}

fn insert_blank_keyframe(doc: &mut AniDoc, params: &Value) -> Out {
    at(doc, params, edit::insert_blank_keyframe)
}

fn clear_keyframe(doc: &mut AniDoc, params: &Value) -> Out {
    at(doc, params, edit::clear_keyframe)
}

fn create_classic_tween(doc: &mut AniDoc, params: &Value) -> Out {
    at(doc, params, edit::create_classic_tween)
}

fn create_shape_tween(doc: &mut AniDoc, params: &Value) -> Out {
    at(doc, params, edit::create_shape_tween)
}

fn remove_tween(doc: &mut AniDoc, params: &Value) -> Out {
    at(doc, params, edit::remove_tween)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Counted {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    count: u32,
}

fn insert_frames(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Counted = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    edit::insert_frames(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?, p.count).map_err(|e| failed(&e))?;
    ok(json!({ "layer": p.layer, "frame": p.frame, "count": p.count }))
}

fn remove_frames(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Counted = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    edit::remove_frames(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?, p.count).map_err(|e| failed(&e))?;
    ok(json!({ "layer": p.layer, "frame": p.frame, "count": p.count }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ease {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    acceleration: i32,
}

fn set_ease(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Ease = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    edit::set_classic_ease(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?, p.acceleration.clamp(-100, 100)).map_err(|e| failed(&e))?;
    ok(json!({ "layer": p.layer, "frame": p.frame, "acceleration": p.acceleration.clamp(-100, 100) }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Name {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    #[serde(rename = "label")]
    text: Option<String>,
}

fn set_label(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Name = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let label = p.text.as_deref().filter(|l| !l.is_empty()).map(|l| (l, LabelType::Name));
    edit::set_frame_label(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?, label).map_err(|e| failed(&e))?;
    ok(json!({ "layer": p.layer, "frame": p.frame, "label": p.text }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Place {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    symbol: String,
    x: Option<f64>,
    y: Option<f64>,
}

fn add_instance(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Place = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let matrix = Affine::translate((p.x.unwrap_or(0.0), p.y.unwrap_or(0.0)));
    let index = edit::add_symbol_instance(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?, &p.symbol, matrix).map_err(|e| failed(&e))?;
    ok(json!({ "element": index }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remove {
    scene: Option<usize>,
    layer: usize,
    frame: u32,
    indices: Vec<usize>,
}

fn remove_elements(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Remove = parse_params(params)?;
    let id = timeline(doc, p.scene)?;
    let removed = edit::remove_elements(&mut doc.editor.document, &id, p.layer, zero_based(p.frame)?, &p.indices).map_err(|e| failed(&e))?;
    ok(json!({ "removed": removed.len() }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rename {
    name: String,
    new_name: String,
}

fn rename_item(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Rename = parse_params(params)?;
    edit::rename_library_item(&mut doc.editor.document, &p.name, &p.new_name).map_err(|e| failed(&e))?;
    ok(json!({ "name": p.new_name }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

fn remove_item(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Named = parse_params(params)?;
    let item = edit::remove_library_item(&mut doc.editor.document, &p.name).map_err(|e| failed(&e))?;
    ok(json!({ "removed": item.name() }))
}

fn duplicate_symbol(doc: &mut AniDoc, params: &Value) -> Out {
    let p: Rename = parse_params(params)?;
    edit::duplicate_symbol(&mut doc.editor.document, &p.name, &p.new_name).map_err(|e| failed(&e))?;
    ok(json!({ "name": p.new_name }))
}

#[cfg(test)]
mod tests {
    use anim_session::{CommandRegistry, Session};

    use super::*;

    fn session() -> Session<AniDoc> {
        let mut registry = CommandRegistry::with_history_commands();
        registry.register_all(all()).unwrap();
        Session::new(AniDoc::default(), registry)
    }

    #[test]
    fn every_schema_is_valid_json() {
        for spec in all() {
            let text = spec.input_schema.unwrap();
            serde_json::from_str::<Value>(text).unwrap_or_else(|e| panic!("{}: {e}", spec.id));
        }
    }

    #[test]
    fn colours_parse() {
        assert_eq!(parse_color("#ff8000").unwrap(), Rgba8 { r: 255, g: 128, b: 0, a: 255 });
        assert_eq!(parse_color("00000080").unwrap().a, 128);
        assert!(parse_color("#12").is_err());
        assert!(parse_color("#gg0000").is_err());
    }

    #[test]
    fn editing_is_undoable_and_errors_change_nothing() {
        let mut s = session();
        s.execute("scene.add", &json!({ "name": "Intro" })).unwrap();
        assert_eq!(s.doc().editor.document.scenes.len(), 2);
        s.execute("layer.add", &json!({ "scene": 1, "name": "bg" })).unwrap();
        s.execute("timeline.insert_keyframe", &json!({ "scene": 1, "layer": 0, "frame": 12 })).unwrap();
        assert!(s.doc().editor.document.scenes[1].layers[0].frames.iter().any(|f| f.index == 11));
        let before = s.doc().editor.document.clone();
        assert!(s.execute("timeline.insert_keyframe", &json!({ "layer": 99, "frame": 3 })).is_err());
        assert!(s.execute("timeline.insert_keyframe", &json!({ "layer": 0, "frame": 0 })).is_err());
        assert!(s.execute("scene.rename", &json!({ "scene": 0, "name": "", "extra": 1 })).is_err());
        assert_eq!(s.doc().editor.document, before);
        s.undo();
        assert_eq!(s.doc().editor.document.scenes[1].layers[0].frames.iter().filter(|f| f.index == 11).count(), 0);
    }

    #[test]
    fn stage_settings_are_validated() {
        let mut s = session();
        s.execute("document.set_stage", &json!({ "width": 1920, "height": 1080, "frame_rate": 30, "background": "#112233" })).unwrap();
        let d = &s.doc().editor.document;
        assert_eq!((d.width, d.height, d.frame_rate), (1920.0, 1080.0, 30.0));
        assert_eq!(d.background_color, Rgba8 { r: 0x11, g: 0x22, b: 0x33, a: 255 });
        assert!(s.execute("document.set_stage", &json!({ "width": -1 })).is_err());
        assert!(s.execute("document.set_stage", &json!({ "frame_rate": 0 })).is_err());
    }
}
