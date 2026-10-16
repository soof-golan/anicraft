//! The Model Context Protocol face of the tools.
//!
//! # Architecture
//!
//! ```text
//!  MCP client ──► anim-session McpServer ──► CommandRegistry<D> ──► this module's commands
//!                                                                      │ build tool events
//!                                                                      ▼
//!                                              Editor::send / run_commands ──► tools (pure) ──► ToolCommand
//!                                                                                                   │ apply
//!                                                                                                   ▼
//!                                                                                                Document
//! ```
//!
//! Every MCP tool is a [`CommandSpec`] registered next to the application's menu commands, so anything a person
//! can do with a tool an agent can do too, and each call is one undo step through the session's history. The
//! commands come in two layers that share one pipeline:
//!
//! * **Raw tools**: `tools_list` describes the sixteen tools, `tools_use` feeds a JSON event script (pointer, key and
//!   text events) to a tool exactly as a person would, `tools_select` and `tools_options` choose the tool and its
//!   options, `tools_state` and `stage_elements` read back what is selected and on the stage.
//! * **Typed façades** for the common jobs, so a model does not have to synthesise pointer events: `draw_rectangle`,
//!   `draw_oval`, `draw_line`, `draw_path`, `draw_freehand`, `draw_text`, `select_at`, `select_rect`, `select_all`,
//!   `select_none`, `transform_move`, `transform_scale`, `transform_rotate`, `transform_skew`, `transform_matrix`,
//!   `edit_delete`, `edit_duplicate`, `edit_enter`, `edit_exit`, `paint_bucket`, `paint_ink`, `paint_erase`,
//!   `paint_eyedrop` and `editor_goto`. Each builds the events of the matching tool and runs them through
//!   [`Editor::send`], so the façade and the mouse behave identically (merge-mode cutting, object drawing, hit
//!   testing, locked layers and all).
//!
//! The host provides the document by implementing [`ToolHost`]; [`Editor`] implements it already, which is what
//! the headless server in `examples/tools_mcp.rs` uses. A command that fails leaves the editor as it was.
//!
//! Colours are `#rgb`, `#rrggbb` or `#rrggbbaa`; `"none"` switches a fill or stroke off. Coordinates are stage
//! pixels, y down; angles are degrees.

#![allow(clippy::needless_raw_string_hashes, reason = "the JSON schemas are easier to read and edit with uniform raw string delimiters")]
#![allow(clippy::redundant_closure_for_method_calls, reason = "method paths do not satisfy the higher-ranked lifetime of with_context")]

use std::fmt::Display;

use anim_core::{Affine, BezPath, Point, Rect, Rgba8, Vec2};
use anim_session::{CommandError, CommandSpec, SizeHint, parse_params};
use anim_shape::{EdgeShape, FillStyle, PathInput, StrokeStyle};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::command::ToolCommand;
use crate::context::ToolContext;
use crate::editor::Editor;
use crate::event::{Modifiers, PointerEvent, ToolEvent};
use crate::script::{ScriptReport, ToolScript, catalog, run_script};
use crate::selection::{ElementRef, Selection};
use crate::settings::{EraserMode, GapSize, ToolSettings};
use crate::tool::ToolId;
use anim_scene::{Element, TimelineId};

/// Gives the command bodies their editor.
pub trait ToolHost {
    fn editor(&self) -> &Editor;
    fn editor_mut(&mut self) -> &mut Editor;
}

impl ToolHost for Editor {
    fn editor(&self) -> &Editor {
        self
    }

    fn editor_mut(&mut self) -> &mut Editor {
        self
    }
}

impl SizeHint for Editor {
    fn size_hint(&self) -> usize {
        let stats = self.document.stats();
        let objects = anim_core::num::u32_to_usize(stats.elements).saturating_add(anim_core::num::u32_to_usize(stats.keyframes));
        4096_usize.saturating_add(objects.saturating_mul(512))
    }
}

fn fail(e: impl Display) -> CommandError {
    CommandError::failed(e.to_string())
}

/// Runs `f`; when it fails the editor is put back as it was.
fn atomic<T>(editor: &mut Editor, f: impl FnOnce(&mut Editor) -> Result<T, CommandError>) -> Result<T, CommandError> {
    let backup = editor.clone();
    let result = f(editor);
    if result.is_err() {
        *editor = backup;
    }
    result
}

fn run(editor: &mut Editor, script: &ToolScript) -> Result<ScriptReport, CommandError> {
    let report = run_script(editor, script);
    report.error.clone().map_or(Ok(report), |message| Err(CommandError::failed(message)))
}

// ---------------------------------------------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------------------------------------------

fn parse_color(text: &str) -> Result<Rgba8, CommandError> {
    Rgba8::from_hex(text).map_err(|e| CommandError::invalid_params(format!("not a colour: {e:?}")))
}

const fn is_none(text: &str) -> bool {
    text.eq_ignore_ascii_case("none")
}

/// A stroke argument: a colour, `"none"`, or `{color, width}`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StrokeArg {
    Text(String),
    Spec {
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        width: Option<f64>,
    },
}

/// The style keys any drawing command accepts.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Style {
    fill: Option<String>,
    stroke: Option<StrokeArg>,
    object_drawing: Option<bool>,
    corner_radius: Option<f64>,
}

const STYLE_KEYS: [&str; 4] = ["fill", "stroke", "object_drawing", "corner_radius"];

/// Splits the style keys off the parameters.
fn split_style(params: &Value) -> Result<(Style, Value), CommandError> {
    let Some(map) = params.as_object() else { return Ok((Style::default(), params.clone())) };
    let mut style = Map::new();
    let mut rest = Map::new();
    for (k, v) in map {
        if STYLE_KEYS.contains(&k.as_str()) {
            style.insert(k.clone(), v.clone())
        } else {
            rest.insert(k.clone(), v.clone())
        };
    }
    Ok((parse_params(&Value::Object(style))?, Value::Object(rest)))
}

fn apply_style(settings: &mut ToolSettings, style: &Style) -> Result<(), CommandError> {
    if let Some(fill) = &style.fill {
        if is_none(fill) {
            settings.fill_enabled = false;
        } else {
            settings.fill = FillStyle::Solid(parse_color(fill)?);
            settings.fill_enabled = true;
        }
    }
    match &style.stroke {
        Some(StrokeArg::Text(text)) if is_none(text) => settings.stroke_enabled = false,
        Some(StrokeArg::Text(text)) => {
            settings.stroke = StrokeStyle::plain(settings.stroke.weight(), parse_color(text)?);
            settings.stroke_enabled = true;
        }
        Some(StrokeArg::Spec { color, width }) => {
            let current = settings.stroke.solid();
            let color = match color {
                Some(c) => parse_color(c)?,
                None => current.fill.solid_color().unwrap_or(Rgba8::BLACK),
            };
            let width = width.unwrap_or(current.weight);
            if !width.is_finite() || width < 0.0 {
                return Err(CommandError::invalid_params("stroke width must be a non-negative number"));
            }
            settings.stroke = StrokeStyle::plain(width, color);
            settings.stroke_enabled = true;
        }
        None => {}
    }
    if let Some(object) = style.object_drawing {
        settings.object_drawing = object;
    }
    if let Some(radius) = style.corner_radius {
        if !radius.is_finite() || radius < 0.0 {
            return Err(CommandError::invalid_params("corner_radius must be a non-negative number"));
        }
        settings.corner_radius = radius;
    }
    Ok(())
}

/// Runs `f` with the style applied to the tool options, then puts the options back.
fn with_style<T>(editor: &mut Editor, style: &Style, f: impl FnOnce(&mut Editor) -> Result<T, CommandError>) -> Result<T, CommandError> {
    let saved = editor.state.settings.clone();
    let result = apply_style(&mut editor.state.settings, style).and_then(|()| f(editor));
    editor.state.settings = saved;
    result
}

// ---------------------------------------------------------------------------------------------------------------
// Describing the stage
// ---------------------------------------------------------------------------------------------------------------

fn bounds_json(r: Rect) -> Value {
    json!([r.x0, r.y0, r.x1, r.y1])
}

fn element_json(ctx: &ToolContext<'_>, r: &ElementRef) -> Value {
    let mut out = Map::new();
    out.insert("ref".into(), serde_json::to_value(r).unwrap_or(Value::Null));
    let Some(resolved) = ctx.resolve(r) else {
        out.insert("kind".into(), json!("missing"));
        return Value::Object(out);
    };
    out.insert("kind".into(), json!(resolved.element.kind_name()));
    if let Some(b) = ctx.stage_bounds(r) {
        out.insert("bounds".into(), bounds_json(b));
    }
    match resolved.element {
        Element::Shape(s) => {
            out.insert("drawing_object".into(), json!(s.is_drawing_object));
            let fills: Vec<String> = s.shape.fills.iter().filter_map(|f| f.solid_color().map(Rgba8::to_hex_rgb)).collect();
            out.insert("fills".into(), json!(fills));
            out.insert("strokes".into(), json!(s.shape.strokes.len()));
        }
        Element::Text(t) => {
            out.insert("text".into(), json!(t.plain_text()));
        }
        Element::SymbolInstance(i) => {
            out.insert("symbol".into(), json!(i.library_item_name));
        }
        Element::Group(g) => {
            out.insert("members".into(), json!(g.members.len()));
        }
        Element::Bitmap(b) => {
            out.insert("bitmap".into(), json!(b.library_item_name));
        }
        Element::Unknown(_) => {}
    }
    Value::Object(out)
}

fn selection_json(editor: &Editor) -> Value {
    editor.with_context(|ctx| {
        let items: Vec<Value> = ctx.selection.items.iter().map(|r| element_json(ctx, r)).collect();
        json!({ "items": items, "scope": ctx.selection.scope, "bounds": ctx.selection_bounds().map(bounds_json) })
    })
}

fn stage_elements_json(editor: &Editor, layer: Option<usize>, frame: Option<u32>) -> Value {
    editor.with_context(|ctx| {
        let layer_index = layer.unwrap_or(ctx.layer);
        // The parameter and the answer count frames from 1, like the timeline; the model counts from 0.
        let frame_number = frame.map_or(ctx.frame, |f| f.saturating_sub(1));
        let shown = frame_number.saturating_add(1);
        let Some(l) = ctx.layer_ref(layer_index) else { return json!({ "layer": layer_index, "elements": [] }) };
        let Some(k) = l.keyframe_at(frame_number) else { return json!({ "layer": layer_index, "frame": shown, "keyframe": null, "elements": [] }) };
        let elements: Vec<Value> = (0..k.elements.len()).map(|i| element_json(ctx, &ElementRef::top_level(layer_index, k.index, i))).collect();
        json!({ "layer": layer_index, "layer_name": l.name, "frame": shown, "keyframe": k.index, "elements": elements })
    })
}

fn result_of(editor: &Editor, report: &ScriptReport) -> Value {
    let frame_elements = stage_elements_json(editor, None, None);
    json!({
        "applied_commands": report.commands_applied,
        "selection": selection_json(editor),
        "stage": frame_elements,
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Event builders
// ---------------------------------------------------------------------------------------------------------------

fn down(x: f64, y: f64) -> ToolEvent {
    PointerEvent::down(x, y).into()
}

fn drag(x: f64, y: f64) -> ToolEvent {
    PointerEvent::drag(x, y).into()
}

fn up(x: f64, y: f64) -> ToolEvent {
    PointerEvent::up(x, y).into()
}

fn finite_all(values: &[f64]) -> Result<(), CommandError> {
    if values.iter().all(|v| v.is_finite()) { Ok(()) } else { Err(CommandError::invalid_params("coordinates must be finite numbers")) }
}

// ---------------------------------------------------------------------------------------------------------------
// Raw tools
// ---------------------------------------------------------------------------------------------------------------

fn tools_list<D: ToolHost>(_: &mut D, _: &Value) -> Result<Value, CommandError> {
    serde_json::to_value(catalog()).map_err(fail)
}

fn tools_use<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let script: ToolScript = parse_params(params)?;
    let editor = host.editor_mut();
    atomic(editor, |ed| {
        let report = run(ed, &script)?;
        let mut value = serde_json::to_value(&report).map_err(fail)?;
        if let Value::Object(map) = &mut value {
            map.insert("stage".into(), stage_elements_json(ed, None, None));
        }
        Ok(value)
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectToolParams {
    tool: ToolId,
}

fn tools_select<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: SelectToolParams = parse_params(params)?;
    host.editor_mut().select_tool(p.tool);
    Ok(json!({ "active_tool": p.tool, "label": p.tool.label() }))
}

/// Merges `patch` into `target` (RFC 7386 merge patch).
fn merge_patch(target: &mut Value, patch: &Value) {
    match patch {
        Value::Object(entries) => {
            if !target.is_object() {
                *target = Value::Object(Map::new());
            }
            if let Value::Object(map) = target {
                for (k, v) in entries {
                    if v.is_null() {
                        map.remove(k);
                    } else {
                        merge_patch(map.entry(k.clone()).or_insert(Value::Null), v);
                    }
                }
            }
        }
        other => *target = other.clone(),
    }
}

fn tools_options<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let editor = host.editor_mut();
    let mut current = serde_json::to_value(&editor.state.settings).map_err(fail)?;
    if params.as_object().is_some_and(|m| !m.is_empty()) {
        merge_patch(&mut current, params);
        editor.state.settings = serde_json::from_value(current.clone()).map_err(|e| CommandError::invalid_params(e.to_string()))?;
    }
    serde_json::to_value(&editor.state.settings).map_err(fail)
}

#[allow(clippy::unnecessary_wraps, reason = "must match the RunFn signature")]
fn tools_state<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    let editor = host.editor();
    let doc = &editor.document;
    let layers: Vec<Value> = doc
        .timeline(&editor.state.timeline)
        .map(|t| {
            t.layers
                .iter()
                .enumerate()
                .map(|(i, l)| json!({ "index": i, "name": l.name, "locked": l.locked, "visible": l.visible, "keyframes": l.frames.len() }))
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "active_tool": editor.tools.active(),
        "timeline": editor.state.timeline,
        "layer": editor.state.layer,
        "frame": editor.state.frame,
        "layers": layers,
        "document": { "width": doc.width, "height": doc.height, "frame_rate": doc.frame_rate, "scenes": doc.scenes.len() },
        "selection": selection_json(editor),
        "settings": editor.state.settings,
        "view": editor.state.view,
        "text_editing": editor.state.text_editing,
        "overlay": editor.overlay(),
    }))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ElementsParams {
    layer: Option<usize>,
    frame: Option<u32>,
}

fn stage_elements<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: ElementsParams = parse_params(params)?;
    Ok(stage_elements_json(host.editor(), p.layer, p.frame))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct GotoParams {
    scene: Option<usize>,
    symbol: Option<String>,
    layer: Option<usize>,
    frame: Option<u32>,
}

fn editor_goto<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: GotoParams = parse_params(params)?;
    let editor = host.editor_mut();
    let timeline = match (&p.scene, &p.symbol) {
        (Some(_), Some(_)) => return Err(CommandError::invalid_params("give either scene or symbol")),
        (Some(i), None) => TimelineId::Scene(*i),
        (None, Some(name)) => TimelineId::Symbol(name.clone()),
        (None, None) => editor.state.timeline.clone(),
    };
    let Some(t) = editor.document.timeline(&timeline) else { return Err(CommandError::invalid_params("no such timeline")) };
    let layer = p.layer.unwrap_or(if timeline == editor.state.timeline { editor.state.layer } else { 0 });
    if layer >= t.layers.len() {
        return Err(CommandError::invalid_params(format!("the timeline has {} layers", t.layers.len())));
    }
    if timeline != editor.state.timeline {
        editor.state.timeline_stack.clear();
        editor.state.in_place_stack.clear();
        editor.state.in_place = None;
        editor.state.edit_range = None;
        editor.state.selection = Selection::empty();
    }
    // A scene that is switched to is the scene the document has open (the one a saved file reopens on).
    if let TimelineId::Scene(i) = &timeline {
        editor.document.current_timeline = *i;
    }
    editor.state.timeline = timeline;
    editor.state.layer = layer;
    if let Some(frame) = p.frame {
        editor.state.frame = frame.checked_sub(1).ok_or_else(|| CommandError::invalid_params("frames start at 1"))?;
    }
    Ok(json!({ "timeline": editor.state.timeline, "layer": editor.state.layer, "frame": editor.state.frame.saturating_add(1) }))
}

// ---------------------------------------------------------------------------------------------------------------
// Drawing façades
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoxParams {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn draw_box<D: ToolHost>(host: &mut D, params: &Value, tool: ToolId) -> Result<Value, CommandError> {
    let (style, rest) = split_style(params)?;
    let p: BoxParams = parse_params(&rest)?;
    finite_all(&[p.x, p.y, p.width, p.height])?;
    if p.width <= 0.0 || p.height <= 0.0 {
        return Err(CommandError::invalid_params("width and height must be positive"));
    }
    let editor = host.editor_mut();
    atomic(editor, |ed| {
        with_style(ed, &style, |ed| {
            let script = ToolScript {
                tool: Some(tool),
                events: vec![down(p.x, p.y), drag(p.x + p.width / 2.0, p.y + p.height / 2.0), up(p.x + p.width, p.y + p.height)],
            };
            let report = run(ed, &script)?;
            Ok(result_of(ed, &report))
        })
    })
}

fn draw_rectangle<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    draw_box(host, params, ToolId::Rectangle)
}

fn draw_oval<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    draw_box(host, params, ToolId::Oval)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LineParams {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
}

fn draw_line<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let (style, rest) = split_style(params)?;
    let p: LineParams = parse_params(&rest)?;
    finite_all(&[p.x1, p.y1, p.x2, p.y2])?;
    atomic(host.editor_mut(), |ed| {
        with_style(ed, &style, |ed| {
            let script = ToolScript {
                tool: Some(ToolId::Line),
                events: vec![down(p.x1, p.y1), drag(f64::midpoint(p.x1, p.x2), f64::midpoint(p.y1, p.y2)), up(p.x2, p.y2)],
            };
            let report = run(ed, &script)?;
            Ok(result_of(ed, &report))
        })
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    /// SVG path data (`M 0 0 L 10 10 C ... Z`).
    d: Option<String>,
    /// Or a polyline.
    points: Option<Vec<[f64; 2]>>,
    #[serde(default)]
    closed: bool,
}

fn draw_path<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let (style, rest) = split_style(params)?;
    let p: PathParams = parse_params(&rest)?;
    let path = match (&p.d, &p.points) {
        (Some(d), None) => BezPath::from_svg(d).map_err(|e| CommandError::invalid_params(format!("bad SVG path data: {e}")))?,
        (None, Some(points)) => {
            let mut path = BezPath::new();
            for (i, [x, y]) in points.iter().enumerate() {
                finite_all(&[*x, *y])?;
                if i == 0 { path.move_to((*x, *y)) } else { path.line_to((*x, *y)) }
            }
            if p.closed {
                path.close_path();
            }
            path
        }
        _ => return Err(CommandError::invalid_params("give exactly one of d or points")),
    };
    if path.elements().iter().any(|el| el.end_point().is_some_and(|pt| !pt.x.is_finite() || !pt.y.is_finite())) {
        return Err(CommandError::invalid_params("the path has non-finite coordinates"));
    }
    atomic(host.editor_mut(), |ed| {
        with_style(ed, &style, |ed| {
            let settings = &ed.state.settings;
            let shape = EdgeShape::from_paths(&[PathInput::new(path, settings.active_fill(), settings.active_stroke())]);
            if shape.is_empty() {
                return Err(CommandError::failed("the path draws nothing"));
            }
            let command = ToolCommand::DrawShape { layer: ed.state.layer, frame: ed.state.frame, shape, object_drawing: settings.object_drawing };
            ed.run_commands(&[command]).map_err(fail)?;
            Ok(json!({ "selection": selection_json(ed), "stage": stage_elements_json(ed, None, None) }))
        })
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FreehandParams {
    points: Vec<[f64; 2]>,
    /// `pencil` (default) or `brush`.
    tool: Option<String>,
    /// Pen pressure per point for the brush (0 to 1).
    pressure: Option<Vec<f32>>,
}

fn draw_freehand<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let (style, rest) = split_style(params)?;
    let p: FreehandParams = parse_params(&rest)?;
    if p.points.len() < 2 {
        return Err(CommandError::invalid_params("a stroke needs at least two points"));
    }
    let tool = match p.tool.as_deref() {
        None | Some("pencil") => ToolId::Pencil,
        Some("brush") => ToolId::Brush,
        Some(other) => return Err(CommandError::invalid_params(format!("unknown freehand tool {other:?}; use pencil or brush"))),
    };
    let last = p.points.len().saturating_sub(1);
    let mut events = Vec::with_capacity(p.points.len());
    for (i, [x, y]) in p.points.iter().enumerate() {
        finite_all(&[*x, *y])?;
        let pressure = p.pressure.as_ref().and_then(|v| v.get(i)).copied().unwrap_or(1.0);
        let event = if i == 0 {
            PointerEvent::down(*x, *y)
        } else if i == last {
            PointerEvent::up(*x, *y)
        } else {
            PointerEvent::drag(*x, *y)
        };
        events.push(event.with_pressure(pressure).into());
    }
    atomic(host.editor_mut(), |ed| {
        with_style(ed, &style, |ed| {
            let report = run(ed, &ToolScript { tool: Some(tool), events })?;
            Ok(result_of(ed, &report))
        })
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextParams {
    x: f64,
    y: f64,
    text: String,
    /// Field width; the field grows with its text when absent.
    width: Option<f64>,
    size: Option<f32>,
    face: Option<String>,
    color: Option<String>,
    bold: Option<bool>,
    italic: Option<bool>,
}

fn draw_text<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: TextParams = parse_params(params)?;
    finite_all(&[p.x, p.y])?;
    if p.text.is_empty() {
        return Err(CommandError::invalid_params("text must not be empty"));
    }
    let color = p.color.as_deref().map(parse_color).transpose()?;
    atomic(host.editor_mut(), |ed| {
        let saved = ed.state.settings.clone();
        {
            let s = &mut ed.state.settings.text;
            if let Some(size) = p.size {
                s.size = size;
            }
            if let Some(face) = &p.face {
                s.face.clone_from(face);
            }
            if let Some(c) = color {
                s.color = c;
            }
            if let Some(b) = p.bold {
                s.bold = b;
            }
            if let Some(i) = p.italic {
                s.italic = i;
            }
        }
        let mut events = vec![down(p.x, p.y)];
        if let Some(width) = p.width {
            events.push(drag(p.x + width / 2.0, p.y));
            events.push(up(p.x + width, p.y));
        } else {
            events.push(up(p.x, p.y));
        }
        events.push(ToolEvent::Text { text: p.text.clone() });
        events.push(crate::event::KeyEvent::press(crate::event::Key::Escape).into());
        let result = run(ed, &ToolScript { tool: Some(ToolId::Text), events });
        ed.state.settings = saved;
        let report = result?;
        Ok(result_of(ed, &report))
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Selection and editing
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectAtParams {
    x: f64,
    y: f64,
    #[serde(default)]
    add: bool,
}

fn select_at<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: SelectAtParams = parse_params(params)?;
    finite_all(&[p.x, p.y])?;
    let modifiers = if p.add { Modifiers::SHIFT } else { Modifiers::NONE };
    let editor = host.editor_mut();
    atomic(editor, |ed| {
        let events = vec![PointerEvent::down(p.x, p.y).with_modifiers(modifiers).into(), PointerEvent::up(p.x, p.y).with_modifiers(modifiers).into()];
        run(ed, &ToolScript { tool: Some(ToolId::Selection), events })?;
        Ok(selection_json(ed))
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectRectParams {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    #[serde(default)]
    add: bool,
}

fn select_rect<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: SelectRectParams = parse_params(params)?;
    finite_all(&[p.x, p.y, p.width, p.height])?;
    let modifiers = if p.add { Modifiers::SHIFT } else { Modifiers::NONE };
    atomic(host.editor_mut(), |ed| {
        let events = vec![
            PointerEvent::down(p.x, p.y).with_modifiers(modifiers).into(),
            PointerEvent::drag(p.x + p.width / 2.0, p.y + p.height / 2.0).with_modifiers(modifiers).into(),
            PointerEvent::up(p.x + p.width, p.y + p.height).with_modifiers(modifiers).into(),
        ];
        run(ed, &ToolScript { tool: Some(ToolId::Selection), events })?;
        Ok(selection_json(ed))
    })
}

fn select_all<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    atomic(host.editor_mut(), |ed| {
        let events = vec![crate::event::KeyEvent::press(crate::event::Key::Char('a')).with_modifiers(Modifiers::CTRL).into()];
        run(ed, &ToolScript { tool: Some(ToolId::Selection), events })?;
        Ok(selection_json(ed))
    })
}

fn select_none<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    let editor = host.editor_mut();
    let selection = editor.state.selection.with_items(Vec::new());
    editor.run_commands(&[ToolCommand::SetSelection { selection }]).map_err(fail)?;
    Ok(selection_json(editor))
}

fn require_selection(editor: &Editor) -> Result<Rect, CommandError> {
    editor.with_context(|ctx| ctx.selection_bounds()).ok_or_else(|| CommandError::failed("nothing is selected; use select_at, select_rect or select_all first"))
}

fn transform_with(editor: &mut Editor, matrix: Affine) -> Result<Value, CommandError> {
    if !matrix.as_coeffs().iter().all(|c| c.is_finite()) {
        return Err(CommandError::invalid_params("the transform is not finite"));
    }
    atomic(editor, |ed| {
        ed.run_commands(&[ToolCommand::TransformSelection { matrix }]).map_err(fail)?;
        Ok(selection_json(ed))
    })
}

/// `matrix` applied about `origin` (the centre of the selection when absent).
fn about(editor: &Editor, origin: Option<[f64; 2]>, matrix: Affine) -> Result<Affine, CommandError> {
    let bounds = require_selection(editor)?;
    let o = origin.map_or_else(|| bounds.center(), |[x, y]| Point::new(x, y));
    finite_all(&[o.x, o.y])?;
    Ok(Affine::translate(o.to_vec2()) * matrix * Affine::translate(-o.to_vec2()))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveParams {
    dx: f64,
    dy: f64,
}

fn transform_move<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: MoveParams = parse_params(params)?;
    finite_all(&[p.dx, p.dy])?;
    let editor = host.editor_mut();
    require_selection(editor)?;
    transform_with(editor, Affine::translate(Vec2::new(p.dx, p.dy)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScaleParams {
    sx: f64,
    sy: Option<f64>,
    origin: Option<[f64; 2]>,
}

fn transform_scale<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: ScaleParams = parse_params(params)?;
    let sy = p.sy.unwrap_or(p.sx);
    finite_all(&[p.sx, sy])?;
    if p.sx == 0.0 || sy == 0.0 {
        return Err(CommandError::invalid_params("a scale of zero would flatten the selection"));
    }
    let editor = host.editor_mut();
    let matrix = about(editor, p.origin, Affine::scale_non_uniform(p.sx, sy))?;
    transform_with(editor, matrix)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RotateParams {
    degrees: f64,
    origin: Option<[f64; 2]>,
}

fn transform_rotate<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: RotateParams = parse_params(params)?;
    finite_all(&[p.degrees])?;
    let editor = host.editor_mut();
    let matrix = about(editor, p.origin, Affine::rotate(p.degrees.to_radians()))?;
    transform_with(editor, matrix)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkewParams {
    #[serde(default)]
    x_degrees: f64,
    #[serde(default)]
    y_degrees: f64,
    origin: Option<[f64; 2]>,
}

fn transform_skew<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: SkewParams = parse_params(params)?;
    finite_all(&[p.x_degrees, p.y_degrees])?;
    if p.x_degrees.abs() >= 89.0 || p.y_degrees.abs() >= 89.0 {
        return Err(CommandError::invalid_params("skew angles must be within 89 degrees"));
    }
    let editor = host.editor_mut();
    let matrix = about(editor, p.origin, Affine::skew(p.x_degrees.to_radians().tan(), p.y_degrees.to_radians().tan()))?;
    transform_with(editor, matrix)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatrixParams {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

fn transform_matrix<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: MatrixParams = parse_params(params)?;
    let editor = host.editor_mut();
    require_selection(editor)?;
    transform_with(editor, Affine::new([p.a, p.b, p.c, p.d, p.e, p.f]))
}

fn edit_delete<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    let editor = host.editor_mut();
    require_selection(editor)?;
    editor.run_commands(&[ToolCommand::DeleteSelection]).map_err(fail)?;
    Ok(stage_elements_json(editor, None, None))
}

fn edit_duplicate<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    let editor = host.editor_mut();
    require_selection(editor)?;
    editor.run_commands(&[ToolCommand::DuplicateSelection]).map_err(fail)?;
    Ok(selection_json(editor))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnterParams {
    x: f64,
    y: f64,
}

fn edit_enter<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: EnterParams = parse_params(params)?;
    finite_all(&[p.x, p.y])?;
    atomic(host.editor_mut(), |ed| {
        let events = vec![PointerEvent::down(p.x, p.y).with_click_count(2).into(), up(p.x, p.y)];
        let report = run(ed, &ToolScript { tool: Some(ToolId::Selection), events })?;
        let entered =
            report.steps.iter().flat_map(|s| s.commands.iter()).any(|c| matches!(c, ToolCommand::EnterGroup { .. } | ToolCommand::EnterSymbol { .. }));
        if !entered {
            return Err(CommandError::failed("there is no group or symbol at that point"));
        }
        Ok(json!({ "timeline": ed.state.timeline, "scope": ed.state.selection.scope, "stage": stage_elements_json(ed, None, None) }))
    })
}

fn edit_exit<D: ToolHost>(host: &mut D, _: &Value) -> Result<Value, CommandError> {
    let editor = host.editor_mut();
    editor.run_commands(&[ToolCommand::ExitScope]).map_err(fail)?;
    Ok(json!({ "timeline": editor.state.timeline, "scope": editor.state.selection.scope }))
}

// ---------------------------------------------------------------------------------------------------------------
// Painting
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BucketParams {
    x: f64,
    y: f64,
    /// Close gaps in the outline: `none`, `small`, `medium` or `large`.
    gap: Option<GapSize>,
}

fn paint_bucket<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let (style, rest) = split_style(params)?;
    let p: BucketParams = parse_params(&rest)?;
    finite_all(&[p.x, p.y])?;
    atomic(host.editor_mut(), |ed| {
        with_style(ed, &style, |ed| {
            if let Some(gap) = p.gap {
                ed.state.settings.bucket_gap = gap;
            }
            let report = run(ed, &ToolScript { tool: Some(ToolId::PaintBucket), events: vec![down(p.x, p.y), up(p.x, p.y)] })?;
            if report.commands_applied == 0 {
                return Err(CommandError::failed("no closed area at that point (try a larger gap)"));
            }
            Ok(result_of(ed, &report))
        })
    })
}

fn paint_ink<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let (style, rest) = split_style(params)?;
    let p: EnterParams = parse_params(&rest)?;
    finite_all(&[p.x, p.y])?;
    atomic(host.editor_mut(), |ed| {
        with_style(ed, &style, |ed| {
            ed.state.settings.stroke_enabled = true;
            let report = run(ed, &ToolScript { tool: Some(ToolId::InkBottle), events: vec![down(p.x, p.y), up(p.x, p.y)] })?;
            if report.commands_applied == 0 {
                return Err(CommandError::failed("no stroke or filled area at that point"));
            }
            Ok(result_of(ed, &report))
        })
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EraseParams {
    points: Vec<[f64; 2]>,
    /// Eraser diameter in pixels.
    size: Option<f64>,
    mode: Option<EraserMode>,
}

fn paint_erase<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: EraseParams = parse_params(params)?;
    if p.points.is_empty() {
        return Err(CommandError::invalid_params("give at least one point"));
    }
    let last = p.points.len().saturating_sub(1);
    let mut events: Vec<ToolEvent> = Vec::with_capacity(p.points.len().saturating_add(1));
    for (i, [x, y]) in p.points.iter().enumerate() {
        finite_all(&[*x, *y])?;
        events.push(if i == 0 {
            down(*x, *y)
        } else if i == last {
            up(*x, *y)
        } else {
            drag(*x, *y)
        });
    }
    if p.points.len() == 1 {
        let [x, y] = p.points.first().copied().unwrap_or([0.0, 0.0]);
        events.push(up(x, y));
    }
    atomic(host.editor_mut(), |ed| {
        let saved = ed.state.settings.clone();
        if let Some(size) = p.size {
            ed.state.settings.eraser_size = size;
        }
        if let Some(mode) = p.mode {
            ed.state.settings.eraser_mode = mode;
        }
        let result = run(ed, &ToolScript { tool: Some(ToolId::Eraser), events });
        ed.state.settings = saved;
        let report = result?;
        Ok(result_of(ed, &report))
    })
}

fn paint_eyedrop<D: ToolHost>(host: &mut D, params: &Value) -> Result<Value, CommandError> {
    let p: EnterParams = parse_params(params)?;
    finite_all(&[p.x, p.y])?;
    atomic(host.editor_mut(), |ed| {
        let report = run(ed, &ToolScript { tool: Some(ToolId::Eyedropper), events: vec![down(p.x, p.y), up(p.x, p.y)] })?;
        if report.commands_applied == 0 {
            return Err(CommandError::failed("no fill or stroke at that point"));
        }
        Ok(json!({ "fill": ed.state.settings.fill, "stroke": ed.state.settings.stroke, "active_tool": ed.tools.active() }))
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------------------------------------------

const SCHEMA_NONE: &str = r#"{"type":"object","properties":{},"additionalProperties":false}"#;

const SCHEMA_TOOLS_USE: &str = r##"{"type":"object","required":["events"],"properties":{"tool":{"type":"string","enum":["selection","subselection","free_transform","line","rectangle","oval","pen","pencil","brush","paint_bucket","ink_bottle","eraser","eyedropper","text","hand","zoom"]},"events":{"type":"array","description":"Pointer, key and text events in order. Positions are stage pixels.","items":{"oneOf":[{"type":"object","required":["type","kind","pos"],"properties":{"type":{"const":"pointer"},"kind":{"enum":["down","move","up","hover"]},"pos":{"type":"object","required":["x","y"],"properties":{"x":{"type":"number"},"y":{"type":"number"}}},"modifiers":{"type":"object","properties":{"shift":{"type":"boolean"},"ctrl":{"type":"boolean"},"alt":{"type":"boolean"}}},"pressure":{"type":"number","minimum":0,"maximum":1},"click_count":{"type":"integer","minimum":1}}},{"type":"object","required":["type","key"],"properties":{"type":{"const":"key"},"key":{"description":"escape, enter, backspace, delete, tab, space, left, right, up, down, home, end, or {\"char\":\"a\"}"},"modifiers":{"type":"object"},"pressed":{"type":"boolean"}}},{"type":"object","required":["type","text"],"properties":{"type":{"const":"text"},"text":{"type":"string"}}},{"type":"object","required":["type","offset","anchor"],"properties":{"type":{"const":"set_caret"},"offset":{"type":"integer","minimum":0},"anchor":{"type":"integer","minimum":0}}},{"type":"object","required":["type"],"properties":{"type":{"const":"cancel"}}}]}}}}"##;

const SCHEMA_TOOLS_SELECT: &str = r#"{"type":"object","required":["tool"],"properties":{"tool":{"type":"string"}},"additionalProperties":false}"#;
const SCHEMA_OPTIONS: &str = r#"{"type":"object","description":"A JSON merge patch of the tool options (fill, stroke, object_drawing, corner_radius, pencil_mode, brush_size, brush_pressure, eraser_mode, eraser_size, bucket_gap, text, hit_tolerance, drag_threshold). Call with no arguments to read them."}"#;
const SCHEMA_ELEMENTS: &str = r#"{"type":"object","properties":{"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based; the playhead by default"}},"additionalProperties":false}"#;
const SCHEMA_GOTO: &str = r#"{"type":"object","properties":{"scene":{"type":"integer","minimum":0},"symbol":{"type":"string","description":"Library name of a symbol to edit"},"layer":{"type":"integer","minimum":0},"frame":{"type":"integer","minimum":1,"description":"1-based playhead position"}},"additionalProperties":false}"#;

const fn schema_box() -> &'static str {
    r##"{"type":"object","required":["x","y","width","height"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number","exclusiveMinimum":0},"height":{"type":"number","exclusiveMinimum":0},"corner_radius":{"type":"number","minimum":0},"fill":{"type":"string"},"stroke":{"oneOf":[{"type":"string"},{"type":"object","properties":{"color":{"type":"string"},"width":{"type":"number","minimum":0}},"additionalProperties":false}]},"object_drawing":{"type":"boolean"}},"additionalProperties":false}"##
}

const SCHEMA_LINE: &str = r##"{"type":"object","required":["x1","y1","x2","y2"],"properties":{"x1":{"type":"number"},"y1":{"type":"number"},"x2":{"type":"number"},"y2":{"type":"number"},"stroke":{"oneOf":[{"type":"string"},{"type":"object","properties":{"color":{"type":"string"},"width":{"type":"number","minimum":0}},"additionalProperties":false}]},"object_drawing":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_PATH: &str = r##"{"type":"object","properties":{"d":{"type":"string","description":"SVG path data"},"points":{"type":"array","items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"closed":{"type":"boolean"},"fill":{"type":"string"},"stroke":{"oneOf":[{"type":"string"},{"type":"object","properties":{"color":{"type":"string"},"width":{"type":"number","minimum":0}},"additionalProperties":false}]},"object_drawing":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_FREEHAND: &str = r##"{"type":"object","required":["points"],"properties":{"points":{"type":"array","minItems":2,"items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"tool":{"enum":["pencil","brush"]},"pressure":{"type":"array","items":{"type":"number","minimum":0,"maximum":1}},"fill":{"type":"string"},"stroke":{"oneOf":[{"type":"string"},{"type":"object"}]},"object_drawing":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_TEXT: &str = r##"{"type":"object","required":["x","y","text"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"text":{"type":"string","description":"Any Unicode, including right-to-left scripts, in typing order"},"width":{"type":"number","exclusiveMinimum":0},"size":{"type":"number"},"face":{"type":"string"},"color":{"type":"string"},"bold":{"type":"boolean"},"italic":{"type":"boolean"}},"additionalProperties":false}"##;
const SCHEMA_SELECT_AT: &str = r#"{"type":"object","required":["x","y"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"add":{"type":"boolean","description":"Add to (or toggle in) the selection"}},"additionalProperties":false}"#;
const SCHEMA_SELECT_RECT: &str = r#"{"type":"object","required":["x","y","width","height"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"width":{"type":"number"},"height":{"type":"number"},"add":{"type":"boolean"}},"additionalProperties":false}"#;
const SCHEMA_MOVE: &str =
    r#"{"type":"object","required":["dx","dy"],"properties":{"dx":{"type":"number"},"dy":{"type":"number"}},"additionalProperties":false}"#;
const SCHEMA_SCALE: &str = r#"{"type":"object","required":["sx"],"properties":{"sx":{"type":"number"},"sy":{"type":"number","description":"Defaults to sx"},"origin":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2,"description":"Fixed point; the selection centre by default"}},"additionalProperties":false}"#;
const SCHEMA_ROTATE: &str = r#"{"type":"object","required":["degrees"],"properties":{"degrees":{"type":"number"},"origin":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"additionalProperties":false}"#;
const SCHEMA_SKEW: &str = r#"{"type":"object","properties":{"x_degrees":{"type":"number"},"y_degrees":{"type":"number"},"origin":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"additionalProperties":false}"#;
const SCHEMA_MATRIX: &str = r#"{"type":"object","required":["a","b","c","d","e","f"],"properties":{"a":{"type":"number"},"b":{"type":"number"},"c":{"type":"number"},"d":{"type":"number"},"e":{"type":"number"},"f":{"type":"number"}},"additionalProperties":false,"description":"Stage-space matrix [a b c d e f] applied to the selection"}"#;
const SCHEMA_POINT: &str = r##"{"type":"object","required":["x","y"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"stroke":{"oneOf":[{"type":"string"},{"type":"object"}]}},"additionalProperties":false}"##;
const SCHEMA_BUCKET: &str = r##"{"type":"object","required":["x","y"],"properties":{"x":{"type":"number"},"y":{"type":"number"},"fill":{"type":"string"},"gap":{"enum":["none","small","medium","large"]}},"additionalProperties":false}"##;
const SCHEMA_ERASE: &str = r#"{"type":"object","required":["points"],"properties":{"points":{"type":"array","minItems":1,"items":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2}},"size":{"type":"number","exclusiveMinimum":0},"mode":{"enum":["normal","fills","lines","selected_fills","inside"]}},"additionalProperties":false}"#;

/// The tool commands, ready for `CommandRegistry::register_all`.
#[must_use]
pub fn commands<D: ToolHost>() -> Vec<CommandSpec<D>> {
    vec![
        CommandSpec::new("tools.list", "List Tools", tools_list::<D>)
            .transient()
            .with_schema(SCHEMA_NONE)
            .describe("List the editing tools with their ids, shortcuts and what they do."),
        CommandSpec::new("tools.use", "Use Tool", tools_use::<D>)
            .with_schema(SCHEMA_TOOLS_USE)
            .describe("Select a tool and feed it pointer, key and text events exactly as a person would; returns the commands the tool produced, the selection and the elements on the stage. The whole script is one undo step."),
        CommandSpec::new("tools.select", "Select Tool", tools_select::<D>).transient().with_schema(SCHEMA_TOOLS_SELECT).describe("Make a tool the active tool."),
        CommandSpec::new("tools.options", "Tool Options", tools_options::<D>)
            .transient()
            .with_schema(SCHEMA_OPTIONS)
            .describe("Read the tool options, or change them with a JSON merge patch (fill, stroke, corner_radius, pencil_mode, brush_size, eraser_mode, bucket_gap, text...)."),
        CommandSpec::new("tools.state", "Tool State", tools_state::<D>)
            .transient()
            .with_schema(SCHEMA_NONE)
            .describe("The active tool, where drawing lands (timeline, layer, frame), the selection, the options, the view and the tool overlay."),
        CommandSpec::new("stage.elements", "Stage Elements", stage_elements::<D>)
            .transient()
            .with_schema(SCHEMA_ELEMENTS)
            .describe("List the elements of the keyframe under the playhead (or of a given layer and frame) with their kinds, bounds and fills."),
        CommandSpec::new("editor.goto", "Go To", editor_goto::<D>)
            .transient()
            .with_schema(SCHEMA_GOTO)
            .describe("Choose the scene or symbol, layer and frame that the drawing tools work on."),
        CommandSpec::new("draw.rectangle", "Draw Rectangle", draw_rectangle::<D>)
            .with_schema(schema_box())
            .describe("Draw a rectangle with the rectangle tool; corner_radius rounds it. Merges with the layer's shape unless object_drawing is true."),
        CommandSpec::new("draw.oval", "Draw Oval", draw_oval::<D>).with_schema(schema_box()).describe("Draw an oval inside the given box with the oval tool."),
        CommandSpec::new("draw.line", "Draw Line", draw_line::<D>).with_schema(SCHEMA_LINE).describe("Draw a straight line with the line tool."),
        CommandSpec::new("draw.path", "Draw Path", draw_path::<D>)
            .with_schema(SCHEMA_PATH)
            .describe("Draw a path from SVG path data (d) or a polyline (points), filled and stroked with the given style."),
        CommandSpec::new("draw.freehand", "Draw Freehand", draw_freehand::<D>)
            .with_schema(SCHEMA_FREEHAND)
            .describe("Draw a freehand stroke through points with the pencil (smoothed line) or the brush (filled, width follows pressure)."),
        CommandSpec::new("draw.text", "Draw Text", draw_text::<D>)
            .with_schema(SCHEMA_TEXT)
            .describe("Place a text field and type text into it. Right-to-left text (Hebrew, Arabic) is entered in typing order."),
        CommandSpec::new("select.at", "Select At", select_at::<D>).transient().with_schema(SCHEMA_SELECT_AT).describe("Click with the selection tool: select the topmost object at a point."),
        CommandSpec::new("select.rect", "Select Rectangle", select_rect::<D>)
            .transient()
            .with_schema(SCHEMA_SELECT_RECT)
            .describe("Drag a marquee with the selection tool: select every object it touches."),
        CommandSpec::new("select.all", "Select All", select_all::<D>).transient().with_schema(SCHEMA_NONE).describe("Select every object on the unlocked layers."),
        CommandSpec::new("select.none", "Select None", select_none::<D>).transient().with_schema(SCHEMA_NONE).describe("Clear the selection."),
        CommandSpec::new("transform.move", "Move Selection", transform_move::<D>).with_schema(SCHEMA_MOVE).describe("Move the selected objects by a distance in stage pixels."),
        CommandSpec::new("transform.scale", "Scale Selection", transform_scale::<D>)
            .with_schema(SCHEMA_SCALE)
            .describe("Scale the selection about a fixed point (its centre by default)."),
        CommandSpec::new("transform.rotate", "Rotate Selection", transform_rotate::<D>)
            .with_schema(SCHEMA_ROTATE)
            .describe("Rotate the selection clockwise by degrees about a fixed point (its centre by default)."),
        CommandSpec::new("transform.skew", "Skew Selection", transform_skew::<D>).with_schema(SCHEMA_SKEW).describe("Skew the selection by angles along x and y."),
        CommandSpec::new("transform.matrix", "Transform Selection", transform_matrix::<D>)
            .with_schema(SCHEMA_MATRIX)
            .describe("Apply a stage-space matrix to the selection."),
        CommandSpec::new("edit.delete", "Delete Selection", edit_delete::<D>).with_schema(SCHEMA_NONE).describe("Delete the selected objects."),
        CommandSpec::new("edit.duplicate", "Duplicate Selection", edit_duplicate::<D>).with_schema(SCHEMA_NONE).describe("Duplicate the selected objects in place; the copies become the selection."),
        CommandSpec::new("edit.enter", "Enter Group or Symbol", edit_enter::<D>)
            .transient()
            .with_schema(SCHEMA_SELECT_AT)
            .describe("Double-click a group or symbol instance at a point to edit inside it."),
        CommandSpec::new("edit.exit", "Exit Group or Symbol", edit_exit::<D>).transient().with_schema(SCHEMA_NONE).describe("Leave the group or symbol being edited."),
        CommandSpec::new("paint.bucket", "Paint Bucket", paint_bucket::<D>)
            .with_schema(SCHEMA_BUCKET)
            .describe("Fill the closed area (or filled region) at a point with a colour; gap closes small gaps in the outline."),
        CommandSpec::new("paint.ink", "Ink Bottle", paint_ink::<D>)
            .with_schema(SCHEMA_POINT)
            .describe("Restyle the stroke at a point, or outline the filled region there, with the stroke style."),
        CommandSpec::new("paint.erase", "Erase", paint_erase::<D>)
            .with_schema(SCHEMA_ERASE)
            .describe("Erase along a path with the eraser; mode limits it to fills, lines or the fill under the first point."),
        CommandSpec::new("paint.eyedrop", "Eyedropper", paint_eyedrop::<D>)
            .transient()
            .with_schema(SCHEMA_POINT)
            .describe("Load the fill or stroke at a point into the tool options and switch to the bucket or ink bottle."),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_is_valid_json_describing_an_object() {
        for spec in commands::<Editor>() {
            let schema = spec.schema_value();
            assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"), "{}", spec.id);
            assert!(spec.input_schema.is_some(), "{} has a schema", spec.id);
            assert!(!spec.description.is_empty(), "{} has a description", spec.id);
        }
    }

    #[test]
    fn merge_patch_follows_rfc_7386() {
        let mut v = json!({"a": 1, "b": {"c": 2, "d": 3}});
        merge_patch(&mut v, &json!({"b": {"c": 5, "d": null}, "e": [1]}));
        assert_eq!(v, json!({"a": 1, "b": {"c": 5}, "e": [1]}));
    }

    #[test]
    fn style_keys_are_split_from_the_rest() {
        let (style, rest) = split_style(&json!({"x": 1, "fill": "#ff0000", "stroke": {"width": 3}, "object_drawing": true})).unwrap();
        assert_eq!(style.fill.as_deref(), Some("#ff0000"));
        assert!(matches!(style.stroke, Some(StrokeArg::Spec { width: Some(w), .. }) if (w - 3.0).abs() < 1.0e-12));
        assert_eq!(rest, json!({"x": 1}));
    }

    #[test]
    fn style_application_validates_colours_and_widths() {
        let mut settings = ToolSettings::default();
        assert!(apply_style(&mut settings, &Style { fill: Some("nonsense".into()), ..Style::default() }).is_err());
        assert!(apply_style(&mut settings, &Style { stroke: Some(StrokeArg::Spec { color: None, width: Some(-1.0) }), ..Style::default() }).is_err());
        apply_style(&mut settings, &Style { fill: Some("none".into()), stroke: Some(StrokeArg::Text("#00f".into())), ..Style::default() }).unwrap();
        assert!(!settings.fill_enabled);
        assert!(settings.stroke_enabled);
    }
}
