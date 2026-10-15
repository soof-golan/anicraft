//! Renders a contact sheet of the tools at work: every cell builds a small document, plays an event script through
//! the real tools (and, for the last cell, MCP calls through a session) and draws the result with `anim-timeline`
//! and `anim-render`, with the tool's overlay on top.
//!
//! ```text
//! cargo run -p anicraft-tools --example screenshots -- OUT.png
//! ```

#![allow(
    clippy::suboptimal_flops,
    clippy::needless_pass_by_value,
    clippy::missing_const_for_fn,
    clippy::items_after_statements,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "a demonstration program: scenario scripts read better as plain arithmetic, and a bad reply from the demo server should fail loudly"
)]

use std::error::Error;
use std::fs::File;
use std::io::BufWriter;

use anicraft_tools::{Editor, Key, KeyEvent, Modifiers, Node as OverlayNode, Overlay, PointerEvent, Role, ToolEvent, ToolId, mcp_commands};
use anim_core::kurbo::{Circle, Shape as _};
use anim_core::{Affine, BezPath, Point, Rect, Rgba8, Size, Vec2};
use anim_display::{DisplayList, Fill, FillRule, Group, Node, Paint, Stroke, StrokeScale, StrokeStyle};
use anim_render::{RenderOptions, Renderer, straight_rgba};
use anim_scene::{Document, Element, Group as SceneGroup, ShapeElement, edit};
use anim_session::{CommandRegistry, McpServer, Session};
use anim_shape::{EdgeShape, FillStyle, PathInput, StrokeStyle as ShapeStroke};
use anim_text::{RichText, Run, TextEngine};
use anim_timeline::Evaluator;
use serde_json::json;

const CELL_W: f64 = 300.0;
const CELL_H: f64 = 210.0;
const CAPTION_H: f64 = 26.0;
const GAP: f64 = 12.0;
const COLUMNS: usize = 5;
const SCALE: f64 = 2.0;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn rgb(r: u8, g: u8, b: u8) -> Rgba8 {
    Rgba8::opaque(r, g, b)
}

fn editor() -> Editor {
    let mut doc = Document::new(CELL_W, CELL_H, 24.0);
    doc.background_color = Rgba8::WHITE;
    Editor::new(doc)
}

fn play(e: &mut Editor, tool: ToolId, events: Vec<ToolEvent>) -> Result<()> {
    e.select_tool(tool);
    for event in &events {
        e.send(event)?;
    }
    Ok(())
}

fn down(x: f64, y: f64) -> ToolEvent {
    PointerEvent::down(x, y).into()
}

fn mv(x: f64, y: f64) -> ToolEvent {
    PointerEvent::drag(x, y).into()
}

fn up(x: f64, y: f64) -> ToolEvent {
    PointerEvent::up(x, y).into()
}

fn with(mods: Modifiers, event: PointerEvent) -> ToolEvent {
    event.with_modifiers(mods).into()
}

fn drag(e: &mut Editor, tool: ToolId, from: (f64, f64), to: (f64, f64)) -> Result<()> {
    play(e, tool, vec![down(from.0, from.1), mv(f64::midpoint(from.0, to.0), f64::midpoint(from.1, to.1)), up(to.0, to.1)])
}

fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.1)
}

fn add_object(e: &mut Editor, path: BezPath, fill: Rgba8) -> Result<()> {
    let mut shape = ShapeElement::new(EdgeShape::from_paths(&[PathInput::new(path, Some(FillStyle::Solid(fill)), None)]));
    shape.is_drawing_object = true;
    edit::add_element(&mut e.document, &e.state.timeline.clone(), 0, 0, Element::Shape(shape))?;
    Ok(())
}

fn set_fill(e: &mut Editor, color: Rgba8) {
    e.state.settings.fill = FillStyle::Solid(color);
    e.state.settings.fill_enabled = true;
}

fn set_stroke(e: &mut Editor, width: f64, color: Rgba8) {
    e.state.settings.stroke = ShapeStroke::plain(width, color);
    e.state.settings.stroke_enabled = true;
}

// ---------------------------------------------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------------------------------------------

fn marquee() -> Result<Editor> {
    let mut e = editor();
    add_object(&mut e, rect_path(30.0, 40.0, 100.0, 100.0), rgb(231, 76, 60))?;
    add_object(&mut e, Circle::new((170.0, 80.0), 34.0).to_path(0.1), rgb(46, 204, 113))?;
    add_object(&mut e, rect_path(200.0, 120.0, 270.0, 180.0), rgb(52, 152, 219))?;
    play(&mut e, ToolId::Selection, vec![down(60.0, 60.0), up(60.0, 60.0)])?;
    play(&mut e, ToolId::Selection, vec![with(Modifiers::SHIFT, PointerEvent::down(140.0, 30.0)), with(Modifiers::SHIFT, PointerEvent::drag(250.0, 140.0))])?;
    Ok(e)
}

fn duplicate_move() -> Result<Editor> {
    let mut e = editor();
    add_object(&mut e, rect_path(40.0, 50.0, 120.0, 120.0), rgb(155, 89, 182))?;
    play(&mut e, ToolId::Selection, vec![down(80.0, 85.0), up(80.0, 85.0)])?;
    play(
        &mut e,
        ToolId::Selection,
        vec![
            with(Modifiers::ALT, PointerEvent::down(80.0, 85.0)),
            with(Modifiers::ALT, PointerEvent::drag(150.0, 90.0)),
            with(Modifiers::ALT, PointerEvent::up(190.0, 100.0)),
        ],
    )?;
    Ok(e)
}

fn bend() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    set_fill(&mut e, rgb(241, 196, 15));
    drag(&mut e, ToolId::Rectangle, (50.0, 70.0), (250.0, 160.0))?;
    play(&mut e, ToolId::Selection, vec![down(150.0, 70.0), mv(150.0, 55.0), mv(150.0, 25.0), up(150.0, 25.0)])?;
    play(&mut e, ToolId::Selection, vec![down(250.0, 115.0), mv(262.0, 115.0), up(285.0, 115.0)])?;
    Ok(e)
}

fn subselect() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    set_fill(&mut e, rgb(26, 188, 156));
    drag(&mut e, ToolId::Rectangle, (50.0, 50.0), (200.0, 150.0))?;
    play(&mut e, ToolId::Selection, vec![down(120.0, 100.0), up(120.0, 100.0)])?;
    play(&mut e, ToolId::Subselection, vec![down(200.0, 150.0), mv(230.0, 170.0), up(250.0, 185.0)])?;
    play(&mut e, ToolId::Subselection, vec![down(50.0, 50.0), mv(70.0, 30.0)])?;
    Ok(e)
}

fn transform() -> Result<Editor> {
    let mut e = editor();
    add_object(&mut e, rect_path(90.0, 70.0, 210.0, 140.0), rgb(230, 126, 34))?;
    play(&mut e, ToolId::Selection, vec![down(150.0, 105.0), up(150.0, 105.0)])?;
    // Outside the bottom right corner is the rotate zone; swing it up.
    play(&mut e, ToolId::FreeTransform, vec![down(222.0, 152.0), mv(235.0, 120.0), mv(240.0, 90.0)])?;
    Ok(e)
}

fn line() -> Result<Editor> {
    let mut e = editor();
    set_stroke(&mut e, 7.0, rgb(44, 62, 80));
    play(&mut e, ToolId::Line, vec![down(40.0, 150.0), mv(120.0, 130.0), up(150.0, 150.0)])?;
    set_stroke(&mut e, 3.0, rgb(192, 57, 43));
    play(
        &mut e,
        ToolId::Line,
        vec![
            with(Modifiers::SHIFT, PointerEvent::down(60.0, 50.0)),
            with(Modifiers::SHIFT, PointerEvent::drag(200.0, 70.0)),
            with(Modifiers::SHIFT, PointerEvent::up(260.0, 80.0)),
        ],
    )?;
    Ok(e)
}

fn rectangle() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.corner_radius = 34.0;
    set_fill(&mut e, rgb(243, 156, 18));
    set_stroke(&mut e, 5.0, rgb(120, 66, 18));
    drag(&mut e, ToolId::Rectangle, (40.0, 40.0), (260.0, 170.0))?;
    Ok(e)
}

fn oval() -> Result<Editor> {
    let mut e = editor();
    set_fill(&mut e, rgb(22, 160, 133));
    set_stroke(&mut e, 4.0, rgb(10, 80, 70));
    drag(&mut e, ToolId::Oval, (40.0, 50.0), (260.0, 160.0))?;
    play(
        &mut e,
        ToolId::Oval,
        vec![
            with(Modifiers::SHIFT, PointerEvent::down(120.0, 80.0)),
            with(Modifiers::SHIFT, PointerEvent::drag(150.0, 100.0)),
            with(Modifiers::SHIFT, PointerEvent::up(180.0, 110.0)),
        ],
    )?;
    Ok(e)
}

fn pen() -> Result<Editor> {
    let mut e = editor();
    set_stroke(&mut e, 3.0, rgb(41, 128, 185));
    e.state.settings.fill_enabled = false;
    play(
        &mut e,
        ToolId::Pen,
        vec![
            down(30.0, 150.0),
            mv(60.0, 90.0),
            up(70.0, 70.0),
            down(130.0, 150.0),
            mv(160.0, 190.0),
            up(170.0, 200.0),
            down(230.0, 90.0),
            up(230.0, 90.0),
            PointerEvent::hover(270.0, 140.0).into(),
        ],
    )?;
    Ok(e)
}

fn pencil() -> Result<Editor> {
    let mut e = editor();
    set_stroke(&mut e, 4.0, rgb(142, 68, 173));
    let mut events = vec![down(20.0, 105.0)];
    for i in 1..70 {
        let x = 20.0 + f64::from(i) * 4.0;
        let wobble = f64::from(i).mul_add(1.7, 0.0).sin() * 2.5;
        events.push(mv(x, (f64::from(i) * 0.14).sin().mul_add(55.0, 105.0) + wobble));
    }
    events.push(up(300.0, 110.0));
    play(&mut e, ToolId::Pencil, events)?;
    Ok(e)
}

fn brush() -> Result<Editor> {
    let mut e = editor();
    set_fill(&mut e, rgb(192, 57, 43));
    e.state.settings.brush_size = 34.0;
    let mut events: Vec<ToolEvent> = vec![PointerEvent::down(30.0, 150.0).with_pressure(0.15).into()];
    for i in 1..40 {
        let t = f64::from(i) / 40.0;
        let pressure = (std::f64::consts::PI * t).sin().max(0.12);
        events
            .push(PointerEvent::drag(30.0 + 240.0 * t, 105.0 + 55.0 * (t * 5.0).cos() * (1.0 - t)).with_pressure(anim_core::num::f64_to_f32(pressure)).into());
    }
    events.push(PointerEvent::up(270.0, 100.0).with_pressure(0.1).into());
    play(&mut e, ToolId::Brush, events)?;
    Ok(e)
}

fn bucket() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.fill_enabled = false;
    set_stroke(&mut e, 3.0, rgb(44, 62, 80));
    drag(&mut e, ToolId::Rectangle, (40.0, 40.0), (190.0, 150.0))?;
    drag(&mut e, ToolId::Oval, (110.0, 70.0), (260.0, 185.0))?;
    for (point, color) in [((60.0, 60.0), rgb(231, 76, 60)), ((150.0, 120.0), rgb(241, 196, 15)), ((230.0, 160.0), rgb(52, 152, 219))] {
        set_fill(&mut e, color);
        play(&mut e, ToolId::PaintBucket, vec![down(point.0, point.1), up(point.0, point.1)])?;
    }
    Ok(e)
}

fn ink() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    set_fill(&mut e, rgb(236, 240, 241));
    drag(&mut e, ToolId::Oval, (50.0, 40.0), (150.0, 170.0))?;
    drag(&mut e, ToolId::Rectangle, (170.0, 60.0), (270.0, 160.0))?;
    set_stroke(&mut e, 9.0, rgb(41, 128, 185));
    play(&mut e, ToolId::InkBottle, vec![down(100.0, 105.0), up(100.0, 105.0)])?;
    set_stroke(&mut e, 5.0, rgb(192, 57, 43));
    play(&mut e, ToolId::InkBottle, vec![down(220.0, 110.0), up(220.0, 110.0)])?;
    Ok(e)
}

fn eraser() -> Result<Editor> {
    let mut e = editor();
    set_fill(&mut e, rgb(39, 174, 96));
    set_stroke(&mut e, 4.0, rgb(20, 90, 50));
    drag(&mut e, ToolId::Oval, (30.0, 30.0), (270.0, 180.0))?;
    e.state.settings.eraser_size = 26.0;
    play(&mut e, ToolId::Eraser, vec![down(20.0, 60.0), mv(80.0, 110.0), mv(130.0, 70.0), mv(180.0, 130.0), mv(230.0, 80.0), up(285.0, 150.0)])?;
    Ok(e)
}

fn text() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.text.size = 30.0;
    e.state.settings.text.color = rgb(44, 62, 80);
    e.select_tool(ToolId::Text);
    for (at, line) in [((24.0, 30.0), "Text: שלום עולם"), ((40.0, 90.0), "مرحبا بالعالم"), ((24.0, 150.0), "abc אבג 123")] {
        e.send(&down(at.0, at.1))?;
        e.send(&up(at.0, at.1))?;
        e.send(&ToolEvent::Text { text: line.to_owned() })?;
        e.send(&KeyEvent::press(Key::Escape).into())?;
    }
    Ok(e)
}

fn merge_mode() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.stroke_enabled = false;
    set_fill(&mut e, rgb(231, 76, 60));
    drag(&mut e, ToolId::Rectangle, (40.0, 50.0), (170.0, 150.0))?;
    set_fill(&mut e, rgb(52, 152, 219));
    drag(&mut e, ToolId::Oval, (110.0, 80.0), (250.0, 180.0))?;
    set_stroke(&mut e, 6.0, rgb(44, 62, 80));
    e.state.settings.fill_enabled = false;
    play(&mut e, ToolId::Line, vec![down(30.0, 170.0), up(270.0, 40.0)])?;
    Ok(e)
}

fn object_drawing() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.object_drawing = true;
    e.state.settings.stroke_enabled = false;
    set_fill(&mut e, rgb(231, 76, 60));
    drag(&mut e, ToolId::Rectangle, (40.0, 50.0), (170.0, 150.0))?;
    set_fill(&mut e, rgb(52, 152, 219));
    drag(&mut e, ToolId::Oval, (100.0, 80.0), (230.0, 180.0))?;
    play(&mut e, ToolId::Selection, vec![down(200.0, 140.0), mv(215.0, 130.0), up(245.0, 105.0)])?;
    Ok(e)
}

fn group_editing() -> Result<Editor> {
    let mut e = editor();
    let mut members = Vec::new();
    for (i, color) in [rgb(231, 76, 60), rgb(241, 196, 15), rgb(46, 204, 113)].into_iter().enumerate() {
        let x = 70.0 + f64::from(u8::try_from(i)?) * 70.0;
        let mut shape =
            ShapeElement::new(EdgeShape::from_paths(&[PathInput::new(Circle::new((x, 105.0), 32.0).to_path(0.1), Some(FillStyle::Solid(color)), None)]));
        shape.is_drawing_object = true;
        members.push(Element::Shape(shape));
    }
    edit::add_element(&mut e.document, &e.state.timeline.clone(), 0, 0, Element::Group(SceneGroup::new(members)))?;
    play(&mut e, ToolId::Selection, vec![down(70.0, 105.0), up(70.0, 105.0)])?;
    play(&mut e, ToolId::Selection, vec![PointerEvent::down(70.0, 105.0).with_click_count(2).into(), up(70.0, 105.0)])?;
    play(&mut e, ToolId::Selection, vec![down(140.0, 105.0), mv(150.0, 85.0), up(160.0, 55.0)])?;
    Ok(e)
}

fn eyedropper() -> Result<Editor> {
    let mut e = editor();
    e.state.settings.object_drawing = true;
    e.state.settings.stroke_enabled = false;
    set_fill(&mut e, rgb(142, 68, 173));
    drag(&mut e, ToolId::Rectangle, (30.0, 60.0), (120.0, 150.0))?;
    set_fill(&mut e, rgb(189, 195, 199));
    drag(&mut e, ToolId::Rectangle, (170.0, 60.0), (260.0, 150.0))?;
    e.state.settings.object_drawing = false;
    // Pick the purple, then the bucket (selected by the eyedropper) paints the grey square.
    play(&mut e, ToolId::Eyedropper, vec![down(70.0, 100.0), up(70.0, 100.0)])?;
    play(&mut e, ToolId::PaintBucket, vec![down(200.0, 100.0), up(200.0, 100.0)])?;
    Ok(e)
}

fn mcp() -> Result<Editor> {
    let mut editor = editor();
    editor.keep_history = false;
    let mut registry = CommandRegistry::with_history_commands();
    registry.register_all(mcp_commands::<Editor>())?;
    let mut session = Session::new(editor, registry);
    let server = McpServer::new("demo", "0");
    let mut call = |name: &str, args: serde_json::Value| -> Result<()> {
        let reply = server
            .handle(&mut session, &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } }))
            .ok_or("no reply")?;
        if reply["result"]["isError"] == json!(true) {
            return Err(format!("{name}: {}", reply["result"]["content"][0]["text"]).into());
        }
        Ok(())
    };
    call(
        "draw_path",
        json!({ "d": "M 40 150 C 40 60 140 60 150 110 C 160 60 260 60 260 150 Z", "fill": "none", "stroke": { "color": "#2c3e50", "width": 4 } }),
    )?;
    call("paint_bucket", json!({ "x": 100, "y": 110, "fill": "#e74c3c" }))?;
    call("paint_bucket", json!({ "x": 210, "y": 110, "fill": "#f1c40f" }))?;
    call("draw_text", json!({ "x": 60, "y": 160, "text": "MCP: שלום", "size": 28, "color": "#34495e" }))?;
    call("select_all", json!({}))?;
    call("transform_rotate", json!({ "degrees": -6 }))?;
    Ok(session.snapshot())
}

// ---------------------------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------------------------

fn solid(color: Rgba8) -> Paint {
    Paint::Solid(color)
}

fn hairline(width: f64) -> StrokeStyle {
    StrokeStyle { width, scale: StrokeScale::None, ..StrokeStyle::default() }
}

fn role_color(role: Role) -> Rgba8 {
    match role {
        Role::Selection | Role::Handle => rgb(31, 111, 235),
        Role::Preview => rgb(240, 140, 0),
        Role::Hover => rgb(22, 163, 74),
        Role::Pivot => rgb(209, 36, 47),
        Role::Guide => rgb(110, 118, 129),
    }
}

fn stroke_node(path: BezPath, color: Rgba8, width: f64) -> Node {
    Node::Stroke(Stroke { transform: Affine::IDENTITY, path, style: hairline(width), paint: solid(color) })
}

fn marker_node(pos: Point, role: Role, filled: bool, circle: bool) -> Vec<Node> {
    let color = role_color(role);
    let half = 3.5;
    let path = if circle { Circle::new(pos, half).to_path(0.05) } else { rect_path(pos.x - half, pos.y - half, pos.x + half, pos.y + half) };
    let mut out = Vec::new();
    out.push(Node::Fill(Fill {
        transform: Affine::IDENTITY,
        path: path.clone(),
        rule: FillRule::NonZero,
        paint: solid(if filled { color } else { Rgba8::WHITE }),
    }));
    out.push(stroke_node(path, color, 1.2));
    out
}

fn overlay_nodes(overlay: &[Overlay]) -> Vec<Node> {
    let mut nodes = Vec::new();
    for item in overlay {
        match item {
            Overlay::Rect { rect, role } => nodes.push(stroke_node(rect.to_path(0.1), role_color(*role), 1.2)),
            Overlay::Marquee { rect } => {
                nodes.push(Node::Fill(Fill {
                    transform: Affine::IDENTITY,
                    path: rect.to_path(0.1),
                    rule: FillRule::NonZero,
                    paint: solid(Rgba8::new(31, 111, 235, 40)),
                }));
                nodes.push(stroke_node(rect.to_path(0.1), rgb(31, 111, 235), 1.0));
            }
            Overlay::Line { a, b, role } => {
                let mut p = BezPath::new();
                p.move_to(*a);
                p.line_to(*b);
                nodes.push(stroke_node(p, role_color(*role), 1.0));
            }
            Overlay::Polyline { points, closed, role } => {
                let mut p = BezPath::new();
                for (i, pt) in points.iter().enumerate() {
                    if i == 0 { p.move_to(*pt) } else { p.line_to(*pt) }
                }
                if *closed {
                    p.close_path();
                }
                nodes.push(stroke_node(p, role_color(*role), 1.3));
            }
            Overlay::Path { path, role, width, .. } => match width {
                Some(w) => nodes.push(Node::Stroke(Stroke {
                    transform: Affine::IDENTITY,
                    path: path.clone(),
                    style: StrokeStyle { width: *w, ..StrokeStyle::default() },
                    paint: solid(Rgba8::new(240, 140, 0, 90)),
                })),
                None => nodes.push(stroke_node(path.clone(), role_color(*role), 1.3)),
            },
            Overlay::Nodes { nodes: anchors, role, .. } => {
                for OverlayNode { pos, control_in, control_out, selected } in anchors {
                    for control in [control_in, control_out].into_iter().flatten() {
                        let mut p = BezPath::new();
                        p.move_to(*pos);
                        p.line_to(*control);
                        nodes.push(stroke_node(p, role_color(Role::Guide), 1.0));
                        nodes.extend(marker_node(*control, *role, true, true));
                    }
                    nodes.extend(marker_node(*pos, *role, *selected, false));
                }
            }
            Overlay::Marker { pos, marker, role, filled } => nodes.extend(marker_node(*pos, *role, *filled, matches!(marker, anicraft_tools::Marker::Circle))),
            Overlay::Crosshair { pos, role } => {
                let mut p = BezPath::new();
                p.move_to(*pos - Vec2::new(8.0, 0.0));
                p.line_to(*pos + Vec2::new(8.0, 0.0));
                p.move_to(*pos - Vec2::new(0.0, 8.0));
                p.line_to(*pos + Vec2::new(0.0, 8.0));
                nodes.push(stroke_node(p, role_color(*role), 1.0));
            }
            Overlay::TextCursor { .. } => {}
        }
    }
    nodes
}

fn label(engine: &mut TextEngine, caption: &str) -> Vec<Node> {
    let style = Run { size: 13.0, color: rgb(30, 41, 59), bold: true, ..Run::default() };
    let layout = engine.layout(&RichText::from_plain(caption, &style), None);
    engine.to_display(&layout)
}

fn cell_nodes(editor: &Editor) -> Vec<Node> {
    let mut evaluator = Evaluator::new(&editor.document);
    let mut nodes =
        vec![Node::Fill(Fill { transform: Affine::IDENTITY, path: rect_path(0.0, 0.0, CELL_W, CELL_H), rule: FillRule::NonZero, paint: solid(Rgba8::WHITE) })];
    nodes.extend(evaluator.frame(0, 0).nodes);
    nodes.extend(overlay_nodes(&editor.overlay()));
    nodes
}

fn main() -> Result<()> {
    let out = std::env::args().nth(1).ok_or("usage: screenshots OUT.png")?;
    type Scenario = fn() -> Result<Editor>;
    let scenarios: [(&str, Scenario); 20] = [
        ("Selection: marquee (shift adds)", marquee),
        ("Selection: alt-drag duplicates", duplicate_move),
        ("Selection: drag an edge to bend it", bend),
        ("Subselection: move points", subselect),
        ("Free Transform: rotate", transform),
        ("Line (shift snaps to 45 deg)", line),
        ("Rectangle: corner radius", rectangle),
        ("Oval (shift: circle)", oval),
        ("Pen: points, handles, rubber band", pen),
        ("Pencil: smoothed", pencil),
        ("Brush: pressure width", brush),
        ("Paint Bucket: enclosed areas", bucket),
        ("Ink Bottle: restyle, outline", ink),
        ("Eraser: through fill and stroke", eraser),
        ("Text: RTL and mixed", text),
        ("Merge mode: shapes cut", merge_mode),
        ("Object drawing: separate", object_drawing),
        ("Group: edit members", group_editing),
        ("Eyedropper + Bucket", eyedropper),
        ("Over MCP: path, bucket, text", mcp),
    ];
    let mut labels = TextEngine::new();
    let mut nodes: Vec<Node> = Vec::new();
    let rows = scenarios.len().div_ceil(COLUMNS);
    for (index, (caption, scenario)) in scenarios.iter().enumerate() {
        let editor = scenario().map_err(|e| format!("{caption}: {e}"))?;
        let (col, row) = (index % COLUMNS, index / COLUMNS);
        let x = f64::from(u8::try_from(col)?).mul_add(CELL_W + GAP, GAP);
        let y = f64::from(u8::try_from(row)?).mul_add(CELL_H + CAPTION_H + GAP, GAP);
        nodes.push(Node::Group(Group { transform: Affine::translate((x, y + 4.0)), children: label(&mut labels, caption), ..Group::default() }));
        let mask = vec![Node::Fill(Fill {
            transform: Affine::IDENTITY,
            path: rect_path(0.0, 0.0, CELL_W, CELL_H),
            rule: FillRule::NonZero,
            paint: solid(Rgba8::BLACK),
        })];
        nodes.push(Node::Group(Group {
            transform: Affine::translate((x, y + CAPTION_H)),
            mask: Some(mask),
            children: cell_nodes(&editor),
            ..Group::default()
        }));
        nodes.push(Node::Stroke(Stroke {
            transform: Affine::translate((x, y + CAPTION_H)),
            path: rect_path(0.0, 0.0, CELL_W, CELL_H),
            style: hairline(1.0),
            paint: solid(rgb(148, 163, 184)),
        }));
    }
    let width = f64::from(u8::try_from(COLUMNS)?).mul_add(CELL_W + GAP, GAP);
    let height = f64::from(u8::try_from(rows)?).mul_add(CELL_H + CAPTION_H + GAP, GAP);
    let sheet = DisplayList { size: Size::new(width, height), background: Some(rgb(226, 232, 240)), nodes };
    let options = RenderOptions::for_stage(sheet.size).with_view(Affine::scale(SCALE)).with_antialias(true);
    let options = RenderOptions { width: options.width.saturating_mul(2), height: options.height.saturating_mul(2), ..options };
    let image = Renderer::new().render(&sheet, &anim_display::ImageStore::new(), &options)?;
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(&out)?), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&straight_rgba(&image))?;
    println!("wrote {out} ({}x{})", image.width, image.height);
    Ok(())
}
