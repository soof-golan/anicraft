//! `anicraft-cli mcp`: the document as a Model Context Protocol server.
//!
//! The architecture is `anim-mcp`'s (see `docs/mcp.md` in anicraft-libs): an rmcp front-end holds only a channel to an
//! actor thread that owns a `SessionHost`, and the host owns the [`AniDoc`], its undo history and these hooks. Nothing
//! is shared and nothing is locked. This module is the application part:
//!
//! * [`commands`]: editing commands (stage, scenes, layers, keyframes, tweens, library), generated into typed MCP tools.
//! * [`AniHooks`]: outline, library, properties, read-only queries, rendering, exporting, opening and saving.
//! * [`serve_stdio`]: the headless actor behind MCP on stdin and stdout.
//!
//! The same hooks and commands drive a running editor, which embeds `anim_mcp::ControlServer` instead.

pub mod commands;
pub mod doc;

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use anicraft_xfl::WriteOptions;
use anim_mcp::{
    AppHooks, Confinement, ControlError, ErrorKind, ExportFormat, ExportRequest, HookOutcome, HookResult, HostInfo, QuerySpec, RenderRequest, Reply,
    SessionHost, Ticket, props, run_job,
};
use anim_scene::{Document, TimelineId};
use anim_session::{CommandRegistry, Session};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use doc::AniDoc;

use crate::export::{self, ExportJob, Format, RenderSpec};
use crate::info;
use crate::project::{Project, evaluator_for, resolve_span, scene_index};

/// The version announced to clients.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The command registry: undo and redo plus [`commands::all`].
///
/// # Panics
/// Never in practice: the built-in commands are checked for duplicate ids by a test.
#[must_use]
pub fn registry() -> CommandRegistry<AniDoc> {
    let mut registry = CommandRegistry::with_history_commands();
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(commands::all()).expect("built-in command ids are unique");
    #[expect(clippy::expect_used, reason = "ids are static and unique; a test registers them")]
    registry.register_all(anicraft_tools::mcp_commands::<AniDoc>()).expect("tool command ids are unique");
    registry
}

/// The application hooks.
#[derive(Debug, Default)]
pub struct AniHooks;

/// The properties `get_property` and `set_property` see: `/stage/width`, `/scenes/0/layers/2/visible`, ...
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PropsView {
    stage: StageProps,
    scenes: Vec<SceneProps>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct StageProps {
    width: f64,
    height: f64,
    frame_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SceneProps {
    name: String,
    layers: Vec<LayerProps>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LayerProps {
    name: String,
    visible: bool,
    locked: bool,
    outline: bool,
}

fn props_view(doc: &Document) -> PropsView {
    PropsView {
        stage: StageProps { width: doc.width, height: doc.height, frame_rate: doc.frame_rate },
        scenes: doc
            .scenes
            .iter()
            .map(|s| SceneProps {
                name: s.name.clone(),
                layers: s.layers.iter().map(|l| LayerProps { name: l.name.clone(), visible: l.visible, locked: l.locked, outline: l.outline }).collect(),
            })
            .collect(),
    }
}

fn apply_props(doc: &mut Document, view: PropsView) -> Result<(), ControlError> {
    let s = &view.stage;
    if !(s.width.is_finite()
        && s.width > 0.0
        && s.height.is_finite()
        && s.height > 0.0
        && s.frame_rate.is_finite()
        && s.frame_rate > 0.0
        && s.frame_rate <= 1000.0)
    {
        return Err(ControlError::invalid("stage width, height and frame_rate must be positive numbers"));
    }
    if view.scenes.len() != doc.scenes.len() {
        return Err(ControlError::invalid("scenes cannot be added or removed through properties; use scene.add and scene.remove"));
    }
    doc.width = s.width;
    doc.height = s.height;
    doc.frame_rate = s.frame_rate;
    for (scene, new) in doc.scenes.iter_mut().zip(view.scenes) {
        if new.layers.len() != scene.layers.len() {
            return Err(ControlError::invalid("layers cannot be added or removed through properties; use layer.add and layer.remove"));
        }
        new.name.clone_into(&mut scene.name);
        for (layer, props) in scene.layers.iter_mut().zip(new.layers) {
            layer.name = props.name;
            layer.visible = props.visible;
            layer.locked = props.locked;
            layer.outline = props.outline;
        }
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RenderOpts {
    scene: Option<usize>,
    /// Render this library symbol instead of a scene.
    symbol: Option<String>,
    transparent: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ExportOpts {
    scene: Option<usize>,
    from: Option<u32>,
    to: Option<u32>,
    fps: Option<f64>,
    scale: Option<f64>,
    transparent: bool,
}

fn options<T: serde::de::DeserializeOwned + Default>(value: &Value) -> Result<T, ControlError> {
    if value.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(value.clone()).map_err(|e| ControlError::invalid(format!("options: {e}")))
}

fn cli_error(e: &crate::CliError) -> ControlError {
    match e {
        crate::CliError::Usage(_) => ControlError::invalid(e.to_string()),
        crate::CliError::Export(anim_export::ExportError::Cancelled) => ControlError::cancelled(),
        _ => ControlError::failed(e.to_string()),
    }
}

fn scene_of(doc: &Document, scene: Option<usize>) -> Result<usize, ControlError> {
    match scene {
        Some(i) if i < doc.scenes.len() => Ok(i),
        Some(i) => Err(ControlError::invalid(format!("no scene {i}; the document has {}", doc.scenes.len()))),
        None => scene_index(doc, None).map_err(|e| cli_error(&e)),
    }
}

/// What an element is, in a few words an agent can act on (the index is its position in the keyframe).
fn element_json(index: usize, element: &anim_scene::Element) -> Value {
    use anim_scene::Element;
    match element {
        Element::Shape(_) => json!({ "index": index, "kind": "shape" }),
        Element::SymbolInstance(i) => json!({ "index": index, "kind": "symbol", "item": i.library_item_name, "name": i.name }),
        Element::Bitmap(b) => json!({ "index": index, "kind": "bitmap", "item": b.library_item_name }),
        Element::Text(t) => json!({ "index": index, "kind": "text", "name": t.instance_name }),
        Element::Group(g) => json!({ "index": index, "kind": "group", "members": g.members.len() }),
        Element::Unknown(_) => json!({ "index": index, "kind": "unknown" }),
    }
}

fn keyframe_json(frame: &anim_scene::Frame) -> Value {
    json!({
        "frame": frame.index.saturating_add(1),
        "duration": frame.duration,
        "tween": format!("{:?}", frame.tween_type),
        "elements": frame.elements.iter().enumerate().map(|(i, e)| element_json(i, e)).collect::<Vec<_>>(),
        "label": frame.label,
        "sound": frame.sound.as_ref().map(|s| s.name.as_str()),
    })
}

impl AppHooks<AniDoc> for AniHooks {
    fn status(&self, session: &Session<AniDoc>) -> Value {
        let d = session.doc().document();
        json!({ "scenes": d.scenes.len(), "stage": [d.width, d.height], "frame_rate": d.frame_rate, "total_frames": d.total_frames(), "frame_numbers": "1-based" })
    }

    fn queries(&self) -> Vec<QuerySpec> {
        let scene = json!({ "type": "integer", "minimum": 0 });
        vec![
            QuerySpec::new("stats", "Counts of scenes, layers, keyframes, elements and library items."),
            QuerySpec::new("validate", "Inconsistencies the model checker finds (dangling references, bad tween spans, ...)."),
            QuerySpec::new("keyframes", "The keyframes in force at a frame on every layer.").with_schema(json!({
                "type": "object", "required": ["frame"], "additionalProperties": false,
                "properties": { "scene": scene, "frame": { "type": "integer", "minimum": 1 } }
            })),
            QuerySpec::new("references", "Where a library item is used (instances, bitmap fills, frame sounds).").with_schema(json!({
                "type": "object", "required": ["name"], "additionalProperties": false, "properties": { "name": { "type": "string" } }
            })),
            QuerySpec::new("sound_cues", "Sounds starting on the keyframes of a scene, with sync and loop mode.").with_schema(json!({
                "type": "object", "additionalProperties": false, "properties": { "scene": scene }
            })),
        ]
    }

    fn query(&mut self, session: &Session<AniDoc>, name: &str, params: &Value, _ticket: &Ticket) -> HookResult {
        let doc = session.doc().document();
        let value = match name {
            "stats" => {
                let s = doc.stats();
                json!({
                    "scenes": s.scenes, "total_frames": s.total_frames, "layers": s.layers, "keyframes": s.keyframes, "elements": s.elements,
                    "symbols": s.symbols, "bitmaps": s.bitmaps, "sounds": s.sounds, "fonts": s.fonts, "videos": s.videos, "folders": s.folders,
                })
            }
            "validate" => json!({ "problems": doc.validate().iter().map(ToString::to_string).collect::<Vec<_>>() }),
            "keyframes" => {
                let frame = params
                    .get("frame")
                    .and_then(Value::as_u64)
                    .and_then(|f| u32::try_from(f).ok())
                    .filter(|f| *f >= 1)
                    .ok_or_else(|| ControlError::invalid("`frame` must be an integer from 1"))?;
                let scene = scene_of(doc, params.get("scene").and_then(Value::as_u64).and_then(|s| usize::try_from(s).ok()))?;
                let timeline = doc.scenes.get(scene).ok_or_else(|| ControlError::invalid("no such scene"))?;
                let rows: Vec<Value> = timeline
                    .keyframes_at(frame.saturating_sub(1))
                    .into_iter()
                    .map(|(layer, k)| {
                        let mut row = keyframe_json(k);
                        if let Some(map) = row.as_object_mut() {
                            map.insert("layer".into(), json!(layer));
                            map.insert("layer_name".into(), json!(timeline.layers.get(layer).map(|l| l.name.as_str())));
                        }
                        row
                    })
                    .collect();
                json!({ "scene": scene, "frame": frame, "keyframes": rows })
            }
            "references" => {
                let name = params.get("name").and_then(Value::as_str).ok_or_else(|| ControlError::invalid("`name` is required"))?;
                let rows: Vec<Value> = doc
                    .references_to(name)
                    .iter()
                    .map(|r| {
                        let timeline = match &r.timeline {
                            TimelineId::Scene(i) => json!({ "scene": i }),
                            TimelineId::Symbol(n) => json!({ "symbol": n }),
                        };
                        json!({ "in": timeline, "layer": r.layer, "frame": r.frame.saturating_add(1), "kind": format!("{:?}", r.kind) })
                    })
                    .collect();
                json!({ "name": name, "uses": rows })
            }
            "sound_cues" => {
                let scene = scene_of(doc, params.get("scene").and_then(Value::as_u64).and_then(|s| usize::try_from(s).ok()))?;
                let cues: Vec<Value> = doc
                    .scenes
                    .get(scene)
                    .map(crate::audio::cues)
                    .unwrap_or_default()
                    .iter()
                    .map(|c| json!({ "sound": c.name, "frame": c.frame.saturating_add(1), "sync": c.sync.xfl_name(), "loop": format!("{:?}", c.looping) }))
                    .collect();
                json!({ "scene": scene, "cues": cues })
            }
            other => return Err(ControlError::new(ErrorKind::NotFound, format!("no query named {other:?}"))),
        };
        Ok(HookOutcome::Ready(Reply::value(value)))
    }

    fn outline(&self, doc: &AniDoc, depth: Option<u32>) -> Result<Value, ControlError> {
        let d = &doc.editor.document;
        let depth = depth.unwrap_or(2);
        let scenes: Vec<Value> = d
            .scenes
            .iter()
            .enumerate()
            .map(|(i, scene)| {
                let t = info::timeline_info(scene);
                let layers: Vec<Value> = if depth >= 1 {
                    scene
                        .layers
                        .iter()
                        .zip(&t.layers)
                        .map(|(layer, l)| {
                            let mut row = json!({ "index": l.index, "name": l.name, "kind": l.kind, "parent": l.parent, "visible": l.visible, "locked": l.locked, "frames": l.frames });
                            if depth >= 2
                                && let Some(map) = row.as_object_mut()
                            {
                                map.insert("keyframes".into(), Value::Array(layer.frames.iter().map(keyframe_json).collect()));
                            }
                            row
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                json!({ "index": i, "name": t.name, "frames": t.frames, "layer_count": t.layers.len(), "labels": t.labels, "layers": layers })
            })
            .collect();
        Ok(json!({
            "stage": { "width": d.width, "height": d.height, "frame_rate": d.frame_rate },
            "conventions": "scene and layer indices are 0-based, layer 0 is the top layer; frame numbers are 1-based",
            "scenes": scenes,
        }))
    }

    fn library(&self, doc: &AniDoc) -> Result<Value, ControlError> {
        serde_json::to_value(info::library_info(&doc.editor.document)).map_err(|e| ControlError::failed(e.to_string()))
    }

    fn get_property(&self, doc: &AniDoc, path: &str) -> Result<Value, ControlError> {
        let view = serde_json::to_value(props_view(&doc.editor.document)).map_err(|e| ControlError::failed(e.to_string()))?;
        props::get(&view, &props::parse_path(path)?).cloned()
    }

    fn set_property(&self, doc: &mut AniDoc, path: &str, value: Value) -> Result<Value, ControlError> {
        let mut view = serde_json::to_value(props_view(&doc.editor.document)).map_err(|e| ControlError::failed(e.to_string()))?;
        let segments = props::parse_path(path)?;
        props::set(&mut view, &segments, value)?;
        let new: PropsView = serde_json::from_value(view).map_err(|e| ControlError::invalid(format!("{path}: {e}")))?;
        apply_props(&mut doc.editor.document, new)?;
        props::get(&serde_json::to_value(props_view(&doc.editor.document)).map_err(|e| ControlError::failed(e.to_string()))?, &segments).cloned()
    }

    fn render(&mut self, doc: &AniDoc, request: &RenderRequest, _ticket: &Ticket) -> HookResult {
        let opts: RenderOpts = options(&request.options)?;
        let mut ev = evaluator_for(&doc.editor.document, &doc.media, false);
        let frame = request.frame.map_or(Ok(1), u32::try_from).map_err(|_| ControlError::invalid("frame numbers start at 1"))?;
        let spec = RenderSpec { scale: f64::from(request.scale), transparent: opts.transparent };
        let (image, what) = if let Some(symbol) = &opts.symbol {
            let frames = doc.editor.document.library.find_symbol(symbol).map_or(1, |s| s.timeline.frame_count());
            let span = resolve_span(frames, Some(frame), Some(frame)).map_err(|e| cli_error(&e))?;
            (export::render_symbol(&mut ev, symbol, span.start, spec).map_err(|e| cli_error(&e))?, json!({ "symbol": symbol, "frame": frame, "of": frames }))
        } else {
            let scene = scene_of(&doc.editor.document, opts.scene)?;
            let total = ev.scene_frames(scene);
            let span = resolve_span(total, Some(frame), Some(frame)).map_err(|e| cli_error(&e))?;
            (export::render_frame(&mut ev, scene, span.start, spec).map_err(|e| cli_error(&e))?, json!({ "scene": scene, "frame": frame, "of": total }))
        };
        let reply = Reply::value(json!({ "rendered": what, "width": image.width, "height": image.height }));
        Ok(HookOutcome::Ready(reply.with_png_rgba(image.width, image.height, &anim_render::straight_rgba(&image))?))
    }

    fn export_formats(&self) -> Vec<ExportFormat> {
        let schema = json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "scene": { "type": "integer", "minimum": 0 },
                "from": { "type": "integer", "minimum": 1, "description": "first frame, 1-based" },
                "to": { "type": "integer", "minimum": 1, "description": "last frame, inclusive" },
                "fps": { "type": "number", "exclusiveMinimum": 0, "maximum": 1000 },
                "scale": { "type": "number", "exclusiveMinimum": 0, "maximum": 64 },
                "transparent": { "type": "boolean", "description": "leave the stage background out (alpha for mov)" }
            }
        });
        Format::ALL
            .into_iter()
            .map(|f| {
                let mut format = ExportFormat::new(f.id(), f.describe(), f.extension());
                f.describe().clone_into(&mut format.description);
                format.schema = schema.clone();
                format
            })
            .collect()
    }

    fn export(&mut self, doc: &AniDoc, request: &ExportRequest, ticket: &Ticket) -> HookResult {
        let opts: ExportOpts = options(&request.options)?;
        let format = Format::parse(&request.format).ok_or_else(|| ControlError::new(ErrorKind::NotFound, format!("no export format {:?}", request.format)))?;
        let scene = scene_of(&doc.editor.document, opts.scene)?;
        let total = evaluator_for(&doc.editor.document, &doc.media, false).scene_frames(scene);
        let mut span = resolve_span(total, opts.from, opts.to).map_err(|e| cli_error(&e))?;
        if format == Format::Svg {
            span = span.start..span.start.saturating_add(1);
        }
        let data = doc.clone();
        let (fps, scale, transparent, out) = (opts.fps, opts.scale.unwrap_or(1.0), opts.transparent, request.path.clone());
        run_job(ticket, "export", move |ticket| {
            let job = ExportJob { doc: data.document(), media: &data.media, scene, span, fps, scale, transparent, format, out };
            let mut progress = |done: u32, count: u32| {
                let fraction = done.saturating_mul(1000).checked_div(count).map_or(1.0, |m| f32::from(u16::try_from(m).unwrap_or(1000)) / 1000.0);
                ticket.progress(fraction, format!("{done}/{count} frames"));
                !ticket.is_cancelled()
            };
            let outcome = export::run(&job, &mut progress).map_err(|e| cli_error(&e))?;
            serde_json::to_value(outcome).map(Reply::value).map_err(|e| ControlError::failed(e.to_string()))
        })
    }

    fn load(&mut self, path: &Path) -> Result<AniDoc, ControlError> {
        Project::open(path).map(AniDoc::from).map_err(|e| ControlError::failed(format!("{}: {e}", path.display())))
    }

    fn new_document(&mut self) -> Result<AniDoc, ControlError> {
        Ok(AniDoc::default())
    }

    fn save(&mut self, doc: &AniDoc, file: &mut File) -> io::Result<()> {
        let (bytes, _report) = anicraft_xfl::write_fla_bytes(&doc.editor.document, &doc.media, &doc.sidecars, &WriteOptions::default()).map_err(io::Error::other)?;
        file.write_all(&bytes)
    }
}

/// Builds the session host: the document, the commands, the hooks and the file confinement.
///
#[must_use]
pub fn host(doc: AniDoc, path: Option<PathBuf>, confinement: Confinement) -> SessionHost<AniDoc, AniHooks> {
    let mut session = Session::new(doc, registry());
    session.set_path(path);
    let info = HostInfo { name: "anicraft-cli".to_owned(), version: VERSION.to_owned() };
    SessionHost::new(info, session, AniHooks).with_confinement(confinement)
}

/// Runs the MCP server on stdin and stdout until the client disconnects.
///
/// `file` is opened first when given. Without `roots` the client cannot read or write any file (renders and
/// previews still come back as images).
///
/// # Errors
/// When the document cannot be read, a root does not exist, or the transport fails.
pub fn serve_stdio(file: Option<&Path>, roots: &[PathBuf]) -> color_eyre::Result<()> {
    let (doc, path) = match file {
        Some(path) => (AniDoc::from(Project::open(path)?), Some(path.to_path_buf())),
        None => (AniDoc::default(), None),
    };
    // Resolved here, so a bad root is reported before the client connects.
    let confinement = Confinement::new(roots, roots).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    let session = anim_mcp::spawn_headless(move || host(doc, path, confinement))?;
    let mcp = anim_mcp::AnimMcp::new(session.backend().clone(), "anicraft-cli", VERSION).with_instructions(INSTRUCTIONS);
    anim_mcp::run_stdio(mcp).map_err(|e| color_eyre::eyre::eyre!("{e}"))?;
    session.shutdown();
    Ok(())
}

/// What the model is told when it connects.
pub const INSTRUCTIONS: &str = "AniCraft edits Adobe Animate documents (.fla / XFL). Call session_status, then inspect_document (scenes, layers, \
keyframes) and get_library. Scene and layer indices are 0-based with layer 0 on top; frame numbers are 1-based like Animate's timeline. Every editing \
command is a tool and one undo step; run_batch applies several atomically. render_frame returns a PNG of any frame so you can look at the result; \
export writes png-seq, gif, mp4, mov, svg, svg-anim, spritesheet or swf. open_document and save_document work inside the allowed folders only.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_builds_with_unique_ids() {
        let r = registry();
        assert!(r.get("scene.add").is_some());
        assert!(r.get("edit.undo").is_some());
    }

    #[test]
    fn properties_round_trip_and_validate() {
        let hooks = AniHooks;
        let mut doc = AniDoc::default();
        assert_eq!(hooks.get_property(&doc, "/stage/width").unwrap(), json!(550.0));
        hooks.set_property(&mut doc, "/stage/width", json!(800)).unwrap();
        assert!((doc.editor.document.width - 800.0).abs() < f64::EPSILON);
        hooks.set_property(&mut doc, "/scenes/0/layers/0/visible", json!(false)).unwrap();
        assert!(!doc.editor.document.scenes[0].layers[0].visible);
        assert!(hooks.set_property(&mut doc, "/stage/width", json!(-3)).is_err());
        assert!(hooks.set_property(&mut doc, "/stage/width", json!("wide")).is_err());
        assert!(hooks.get_property(&doc, "/scenes/9").is_err());
    }
}
