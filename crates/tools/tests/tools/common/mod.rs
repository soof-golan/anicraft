//! Shared helpers for the tool tests: documents to draw on and short ways to drive an editor.

#![allow(dead_code)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::arithmetic_side_effects, reason = "test code")]

use anicraft_tools::{Editor, PointerEvent, ToolCommand, ToolEvent, ToolId, mcp_commands};
use anim_core::{Affine, BezPath, Point, Rgba8};
use anim_scene::{Document, Element, Group, ShapeElement, SymbolKind, TimelineId, edit};
use anim_session::{CommandRegistry, McpServer, Session};
use anim_shape::{EdgeShape, FillStyle, PathInput, StrokeStyle};
use serde_json::{Value, json};

pub const RED: Rgba8 = Rgba8::opaque(255, 0, 0);
pub const BLUE: Rgba8 = Rgba8::opaque(0, 0, 255);

/// A 550x400 document with one layer and a blank keyframe.
pub fn blank_document() -> Document {
    Document::new(550.0, 400.0, 24.0)
}

pub fn editor() -> Editor {
    Editor::new(blank_document())
}

pub fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    anim_core::kurbo::Shape::to_path(&anim_core::Rect::new(x0, y0, x1, y1), 0.1)
}

pub fn rect_shape(x0: f64, y0: f64, x1: f64, y1: f64, fill: Option<Rgba8>, stroke: Option<f64>) -> EdgeShape {
    EdgeShape::from_paths(&[PathInput::new(rect_path(x0, y0, x1, y1), fill.map(FillStyle::Solid), stroke.map(|w| StrokeStyle::plain(w, Rgba8::BLACK)))])
}

/// Adds a drawing-object rectangle to layer 0 at the current frame and returns its index.
pub fn add_object_rect(e: &mut Editor, x0: f64, y0: f64, x1: f64, y1: f64, fill: Rgba8) -> usize {
    let mut shape = ShapeElement::new(rect_shape(x0, y0, x1, y1, Some(fill), None));
    shape.is_drawing_object = true;
    edit::add_element(&mut e.document, &TimelineId::Scene(0), 0, 0, Element::Shape(shape)).unwrap()
}

pub fn add_group(e: &mut Editor, members: Vec<Element>) -> usize {
    edit::add_element(&mut e.document, &TimelineId::Scene(0), 0, 0, Element::Group(Group::new(members))).unwrap()
}

pub fn object_rect(x0: f64, y0: f64, x1: f64, y1: f64, fill: Rgba8) -> Element {
    let mut shape = ShapeElement::new(rect_shape(x0, y0, x1, y1, Some(fill), None));
    shape.is_drawing_object = true;
    Element::Shape(shape)
}

/// Defines a graphic symbol holding a rectangle and places an instance of it on layer 0.
pub fn add_symbol_with_rect(e: &mut Editor, name: &str, rect: (f64, f64, f64, f64), at: (f64, f64)) -> usize {
    e.document.add_symbol(name, SymbolKind::MovieClip).unwrap();
    let id = TimelineId::Symbol(name.to_owned());
    edit::add_element(&mut e.document, &id, 0, 0, object_rect(rect.0, rect.1, rect.2, rect.3, RED)).unwrap();
    edit::add_symbol_instance(&mut e.document, &TimelineId::Scene(0), 0, 0, name, Affine::translate((at.0, at.1))).unwrap()
}

/// Elements of the first keyframe of layer 0 of the current timeline.
pub fn elements(e: &Editor) -> Vec<&Element> {
    e.document.timeline(&e.state.timeline).unwrap().layers[0].frames.iter().flat_map(|f| f.elements.iter()).collect()
}

pub fn shape_at(e: &Editor, index: usize) -> &ShapeElement {
    match elements(e)[index] {
        Element::Shape(s) => s,
        other => panic!("element {index} is a {}", other.kind_name()),
    }
}

pub fn fill_color_at(shape: &EdgeShape, p: Point) -> Option<Rgba8> {
    shape.fill(shape.fill_at(p)?)?.solid_color()
}

#[allow(clippy::needless_pass_by_value)]
pub fn send_all(e: &mut Editor, events: Vec<ToolEvent>) -> Vec<ToolCommand> {
    let mut out = Vec::new();
    for event in &events {
        out.extend(e.send(event).unwrap().commands);
    }
    out
}

pub fn down(x: f64, y: f64) -> ToolEvent {
    PointerEvent::down(x, y).into()
}

pub fn mv(x: f64, y: f64) -> ToolEvent {
    PointerEvent::drag(x, y).into()
}

pub fn up(x: f64, y: f64) -> ToolEvent {
    PointerEvent::up(x, y).into()
}

pub fn hover(x: f64, y: f64) -> ToolEvent {
    PointerEvent::hover(x, y).into()
}

/// Press, a few moves and release.
pub fn drag(e: &mut Editor, tool: ToolId, from: (f64, f64), to: (f64, f64)) -> Vec<ToolCommand> {
    e.select_tool(tool);
    send_all(e, vec![down(from.0, from.1), mv(f64::midpoint(from.0, to.0), f64::midpoint(from.1, to.1)), mv(to.0, to.1), up(to.0, to.1)])
}

pub fn click(e: &mut Editor, tool: ToolId, at: (f64, f64)) -> Vec<ToolCommand> {
    e.select_tool(tool);
    send_all(e, vec![down(at.0, at.1), up(at.0, at.1)])
}

/// A JSON-RPC client on an in-process MCP server that drives an editor through the registered tool commands.
pub struct Client {
    pub server: McpServer,
    pub session: Session<Editor>,
    next_id: u64,
}

impl Client {
    /// A client on the blank editor.
    pub fn new() -> Self {
        Self::on(editor())
    }

    /// A client on `editor`, with history snapshots off.
    pub fn on(mut editor: Editor) -> Self {
        editor.keep_history = false;
        let mut registry = CommandRegistry::with_history_commands();
        registry.register_all(mcp_commands::<Editor>()).unwrap();
        Self { server: McpServer::new("anicraft-tools-test", "0.1.0"), session: Session::new(editor, registry), next_id: 1 }
    }

    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let message = json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params });
        self.server.handle(&mut self.session, &message).expect("a response")
    }

    /// Calls a tool and returns (is_error, structured content or text).
    pub fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let reply = self.rpc("tools/call", json!({ "name": name, "arguments": arguments }));
        let result = &reply["result"];
        assert!(result.is_object(), "{reply}");
        let is_error = result["isError"].as_bool().unwrap();
        let content = result.get("structuredContent").cloned().unwrap_or_else(|| result["content"][0]["text"].clone());
        (is_error, content)
    }

    pub fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (is_error, content) = self.call(name, arguments);
        assert!(!is_error, "{name} failed: {content}");
        content
    }

    pub fn err(&mut self, name: &str, arguments: Value) -> String {
        let (is_error, content) = self.call(name, arguments);
        assert!(is_error, "{name} should fail but returned {content}");
        content.as_str().unwrap_or_default().to_owned()
    }

    pub fn editor(&self) -> &Editor {
        self.session.doc()
    }
}
