//! The MCP face: the tool commands registered in an `anim-session` registry and driven with JSON-RPC.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::panic,
    clippy::too_many_lines,
    clippy::missing_const_for_fn,
    clippy::float_cmp,
    clippy::manual_assert_eq,
    clippy::assert_is_empty,
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    clippy::needless_pass_by_value,
    clippy::doc_markdown,
    clippy::disallowed_types
)]

mod common;

use anicraft_tools::{Editor, mcp_commands};
use anim_session::{CommandRegistry, McpServer, Session};
use common::*;
use proptest::prelude::*;
use serde_json::{Value, json};

fn session() -> (McpServer, Session<Editor>) {
    let mut editor = editor();
    editor.keep_history = false;
    let mut registry = CommandRegistry::with_history_commands();
    registry.register_all(mcp_commands::<Editor>()).unwrap();
    (McpServer::new("anicraft-tools-test", "0.1.0"), Session::new(editor, registry))
}

struct Client {
    server: McpServer,
    session: Session<Editor>,
    next_id: u64,
}

impl Client {
    fn new() -> Self {
        let (server, session) = session();
        Self { server, session, next_id: 1 }
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let message = json!({ "jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params });
        self.server.handle(&mut self.session, &message).expect("a response")
    }

    /// Calls a tool and returns (is_error, structured content or text).
    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let reply = self.rpc("tools/call", json!({ "name": name, "arguments": arguments }));
        let result = &reply["result"];
        assert!(result.is_object(), "{reply}");
        let is_error = result["isError"].as_bool().unwrap();
        let content = result.get("structuredContent").cloned().unwrap_or_else(|| result["content"][0]["text"].clone());
        (is_error, content)
    }

    fn ok(&mut self, name: &str, arguments: Value) -> Value {
        let (is_error, content) = self.call(name, arguments);
        assert!(!is_error, "{name} failed: {content}");
        content
    }

    fn err(&mut self, name: &str, arguments: Value) -> String {
        let (is_error, content) = self.call(name, arguments);
        assert!(is_error, "{name} should fail but returned {content}");
        content.as_str().unwrap_or_default().to_owned()
    }

    fn editor(&self) -> &Editor {
        self.session.doc()
    }

    fn stage(&mut self) -> Vec<Value> {
        self.ok("stage_elements", json!({}))["elements"].as_array().unwrap().clone()
    }
}

#[test]
fn the_tools_are_listed_with_schemas_and_read_only_hints() {
    let mut c = Client::new();
    let reply = c.rpc("tools/list", json!({}));
    let tools = reply["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "tools_list",
        "tools_use",
        "tools_select",
        "tools_options",
        "tools_state",
        "stage_elements",
        "editor_goto",
        "draw_rectangle",
        "draw_oval",
        "draw_line",
        "draw_path",
        "draw_freehand",
        "draw_text",
        "select_at",
        "select_rect",
        "select_all",
        "select_none",
        "transform_move",
        "transform_scale",
        "transform_rotate",
        "transform_skew",
        "transform_matrix",
        "edit_delete",
        "edit_duplicate",
        "edit_enter",
        "edit_exit",
        "paint_bucket",
        "paint_ink",
        "paint_erase",
        "paint_eyedrop",
        "edit_undo",
    ] {
        assert!(names.contains(&expected), "missing {expected}");
    }
    let rectangle = tools.iter().find(|t| t["name"] == "draw_rectangle").unwrap();
    assert_eq!(rectangle["inputSchema"]["required"], json!(["x", "y", "width", "height"]));
    assert_eq!(rectangle["annotations"]["readOnlyHint"], json!(false));
    let state = tools.iter().find(|t| t["name"] == "tools_state").unwrap();
    assert_eq!(state["annotations"]["readOnlyHint"], json!(true));
    assert!(rectangle["description"].as_str().unwrap().contains("rectangle"));
}

#[test]
fn draw_a_rectangle_and_undo_it() {
    let mut c = Client::new();
    let result = c.ok("draw_rectangle", json!({ "x": 50, "y": 40, "width": 200, "height": 100, "fill": "#00ff00", "stroke": { "color": "#000", "width": 3 } }));
    let elements = result["stage"]["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0]["kind"], "shape");
    assert_eq!(elements[0]["fills"], json!(["#00FF00"]));
    let bounds = elements[0]["bounds"].as_array().unwrap();
    assert!((bounds[0].as_f64().unwrap() - 48.5).abs() < 0.2, "stroke widens the bounds: {bounds:?}");
    // The style was only for this call.
    assert_eq!(c.editor().state.settings, anicraft_tools::ToolSettings::default());
    // Undo is the session's: one step per call.
    c.ok("edit_undo", json!({}));
    assert!(c.stage().is_empty());
    c.ok("edit_redo", json!({}));
    assert_eq!(c.stage().len(), 1);
}

#[test]
fn object_drawing_rounding_and_the_other_shapes() {
    let mut c = Client::new();
    c.ok("draw_rectangle", json!({ "x": 10, "y": 10, "width": 100, "height": 60, "corner_radius": 20, "object_drawing": true }));
    c.ok("draw_oval", json!({ "x": 150, "y": 10, "width": 100, "height": 60, "object_drawing": true, "stroke": "none" }));
    c.ok("draw_line", json!({ "x1": 10, "y1": 100, "x2": 300, "y2": 100, "stroke": { "color": "#f00", "width": 5 } }));
    let elements = c.stage();
    assert_eq!(elements.len(), 3);
    assert_eq!(elements[0]["drawing_object"], true);
    assert_eq!(elements[1]["strokes"], 0);
    assert_eq!(elements[2]["kind"], "shape");
}

#[test]
fn draw_path_from_svg_data_and_from_points() {
    let mut c = Client::new();
    c.ok("draw_path", json!({ "d": "M 20 20 C 60 -20 100 60 140 20 L 140 100 Z", "fill": "#ff8800", "stroke": "none", "object_drawing": true }));
    c.ok("draw_path", json!({ "points": [[200, 20], [260, 20], [230, 80]], "closed": true, "fill": "#0088ff", "object_drawing": true }));
    assert_eq!(c.stage().len(), 2);
    let msg = c.err("draw_path", json!({ "d": "M 0 0 L nonsense" }));
    assert!(msg.contains("SVG"), "{msg}");
    assert!(c.err("draw_path", json!({ "d": "M 0 0", "points": [[0, 0]] })).contains("exactly one"));
    assert_eq!(c.stage().len(), 2, "failures change nothing");
}

#[test]
fn select_transform_and_edit() {
    let mut c = Client::new();
    c.ok("draw_rectangle", json!({ "x": 100, "y": 100, "width": 100, "height": 50, "object_drawing": true, "stroke": "none" }));
    c.ok("draw_rectangle", json!({ "x": 300, "y": 100, "width": 60, "height": 60, "object_drawing": true, "stroke": "none", "fill": "#00f" }));
    // Object drawing leaves the new object selected; nothing is selected after clearing.
    assert_eq!(c.ok("tools_state", json!({}))["selection"]["items"].as_array().unwrap().len(), 1);
    c.ok("select_none", json!({}));
    let none = c.err("transform_move", json!({ "dx": 5, "dy": 5 }));
    assert!(none.contains("nothing is selected"), "{none}");
    let sel = c.ok("select_at", json!({ "x": 150, "y": 125 }));
    assert_eq!(sel["items"].as_array().unwrap().len(), 1);
    c.ok("transform_move", json!({ "dx": 30, "dy": -20 }));
    let sel = c.ok("select_none", json!({}));
    assert_eq!(sel["items"], json!([]));
    let sel = c.ok("select_rect", json!({ "x": 0, "y": 0, "width": 500, "height": 400 }));
    assert_eq!(sel["items"].as_array().unwrap().len(), 2);
    c.ok("transform_scale", json!({ "sx": 2 }));
    c.ok("transform_rotate", json!({ "degrees": 90 }));
    let bounds = c.ok("tools_state", json!({}))["selection"]["bounds"].clone();
    let b: Vec<f64> = bounds.as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    // The two rectangles span 230 x 80; scaling by two gives 460 x 160 and the quarter turn swaps the two.
    assert!((b[2] - b[0] - 160.0).abs() < 1.0e-6 && (b[3] - b[1] - 460.0).abs() < 1.0e-6, "{b:?}");
    c.ok("transform_skew", json!({ "x_degrees": 10 }));
    c.ok("transform_matrix", json!({ "a": 1, "b": 0, "c": 0, "d": 1, "e": 1, "f": 1 }));
    assert!(c.err("transform_scale", json!({ "sx": 0 })).contains("zero"));
    assert!(c.err("transform_skew", json!({ "x_degrees": 90 })).contains("89"));
    c.ok("edit_duplicate", json!({}));
    assert_eq!(c.stage().len(), 4);
    c.ok("edit_delete", json!({}));
    assert_eq!(c.stage().len(), 2);
}

#[test]
fn text_in_hebrew_arrives_intact() {
    let mut c = Client::new();
    let result = c.ok("draw_text", json!({ "x": 40, "y": 60, "text": "שלום עולם hello", "size": 36, "color": "#112233" }));
    let elements = result["stage"]["elements"].as_array().unwrap();
    assert_eq!(elements.len(), 1);
    assert_eq!(elements[0]["kind"], "text");
    assert_eq!(elements[0]["text"], "שלום עולם hello");
    assert!(c.err("draw_text", json!({ "x": 0, "y": 0, "text": "" })).contains("empty"));
    // Fixed width field.
    c.ok("draw_text", json!({ "x": 40, "y": 200, "text": "مرحبا", "width": 120 }));
    assert_eq!(c.stage().len(), 2);
    // The options were restored.
    assert_eq!(c.editor().state.settings.text.size, 24.0);
}

#[test]
fn bucket_ink_erase_and_eyedropper() {
    let mut c = Client::new();
    c.ok("draw_rectangle", json!({ "x": 100, "y": 100, "width": 200, "height": 100, "fill": "none", "stroke": { "width": 2 } }));
    assert!(c.err("paint_bucket", json!({ "x": 20, "y": 20 })).contains("no closed area"));
    c.ok("paint_bucket", json!({ "x": 200, "y": 150, "fill": "#3366ff" }));
    let fills = c.stage()[0]["fills"].clone();
    assert_eq!(fills, json!(["#3366FF"]));
    c.ok("paint_ink", json!({ "x": 100, "y": 150, "stroke": { "color": "#ff0000", "width": 6 } }));
    c.ok("paint_erase", json!({ "points": [[90, 150], [310, 150]], "size": 20, "mode": "fills" }));
    let state = c.ok("tools_state", json!({}));
    assert_eq!(state["settings"]["eraser_size"], json!(16.0), "erase options are per call");
    let picked = c.ok("paint_eyedrop", json!({ "x": 200, "y": 110 }));
    assert_eq!(picked["active_tool"], "paint_bucket");
    assert!(c.err("paint_eyedrop", json!({ "x": 900, "y": 900 })).contains("no fill"));
}

#[test]
fn enter_symbols_groups_and_go_to() {
    let mut c = Client::new();
    let mut e = c.session.doc_mut().clone();
    add_symbol_with_rect(&mut e, "box", (0.0, 0.0, 80.0, 60.0), (100.0, 100.0));
    c.session.replace_document(e, None);
    c.ok("select_at", json!({ "x": 140, "y": 130 }));
    let entered = c.ok("edit_enter", json!({ "x": 140, "y": 130 }));
    assert_eq!(entered["timeline"], json!({ "Symbol": "box" }));
    assert_eq!(c.stage().len(), 1);
    c.ok("edit_exit", json!({}));
    assert_eq!(c.ok("tools_state", json!({}))["timeline"], json!({ "Scene": 0 }));
    assert!(c.err("edit_enter", json!({ "x": 500, "y": 350 })).contains("no group"));
    c.ok("editor_goto", json!({ "symbol": "box", "layer": 0, "frame": 1 }));
    assert!(c.err("editor_goto", json!({ "frame": 0 })).contains("start at 1"));
    c.ok("draw_oval", json!({ "x": 10, "y": 10, "width": 30, "height": 30 }));
    assert_eq!(c.stage().len(), 2, "drawing lands inside the symbol");
    assert!(c.err("editor_goto", json!({ "symbol": "missing" })).contains("no such timeline"));
    assert!(c.err("editor_goto", json!({ "scene": 0, "symbol": "box" })).contains("either"));
    assert!(c.err("editor_goto", json!({ "layer": 40 })).contains("layers"));
}

#[test]
fn raw_tool_use_matches_the_facades() {
    let mut c = Client::new();
    let script = json!({
        "tool": "rectangle",
        "events": [
            {"type": "pointer", "kind": "down", "pos": {"x": 10, "y": 10}},
            {"type": "pointer", "kind": "move", "pos": {"x": 60, "y": 40}},
            {"type": "pointer", "kind": "up", "pos": {"x": 90, "y": 60}}
        ]
    });
    let report = c.ok("tools_use", script);
    assert_eq!(report["commands_applied"], 1);
    assert_eq!(report["stage"]["elements"].as_array().unwrap().len(), 1);
    assert_eq!(report["active_tool"], "rectangle");
    // A script that cannot run fails as a whole.
    let bad = json!({ "tool": "selection", "events": [{"type": "pointer", "kind": "down", "pos": {"x": 1, "y": 1}}, {"type": "frobnicate"}] });
    let msg = c.err("tools_use", bad);
    assert!(msg.to_lowercase().contains("invalid") || msg.contains("frobnicate"), "{msg}");
    assert_eq!(c.stage().len(), 1);
}

#[test]
fn tool_options_are_a_merge_patch() {
    let mut c = Client::new();
    let before = c.ok("tools_options", json!({}));
    assert_eq!(before["corner_radius"], json!(0.0));
    let after = c.ok("tools_options", json!({ "corner_radius": 12.5, "text": { "size": 40.0 } }));
    assert_eq!(after["corner_radius"], json!(12.5));
    assert_eq!(after["text"]["size"], json!(40.0));
    assert_eq!(after["text"]["face"], json!("Arial"), "untouched keys survive");
    assert!(c.err("tools_options", json!({ "pencil_mode": "wobbly" })).contains("invalid"));
    assert_eq!(c.ok("tools_options", json!({}))["corner_radius"], json!(12.5), "a rejected patch changes nothing");
    c.ok("tools_select", json!({ "tool": "oval" }));
    assert_eq!(c.ok("tools_state", json!({}))["active_tool"], "oval");
    let listed = c.ok("tools_list", json!({}));
    assert_eq!(listed.as_array().map(Vec::len), None, "arrays come back as text content");
}

#[test]
fn bad_parameters_are_reported_not_applied() {
    let mut c = Client::new();
    assert!(c.err("draw_rectangle", json!({ "x": 1, "y": 1, "width": -5, "height": 10 })).contains("positive"));
    assert!(c.err("draw_rectangle", json!({ "x": 1, "y": 1, "width": 5 })).to_lowercase().contains("height"));
    assert!(c.err("draw_rectangle", json!({ "x": 1, "y": 1, "width": 5, "height": 5, "colour": "red" })).contains("colour"));
    assert!(c.err("draw_rectangle", json!({ "x": 1, "y": 1, "width": 5, "height": 5, "fill": "mauve" })).contains("colour"));
    assert!(c.err("draw_freehand", json!({ "points": [[1, 1]] })).contains("two points"));
    assert!(c.err("draw_freehand", json!({ "points": [[1, 1], [5, 5]], "tool": "crayon" })).contains("crayon"));
    assert!(c.err("paint_erase", json!({ "points": [] })).contains("one point"));
    assert!(c.stage().is_empty());
    let reply = c.rpc("tools/call", json!({ "name": "no_such_tool", "arguments": {} }));
    assert!(reply["error"].is_object());
    // Layers that are locked are not drawn on, and the call says nothing happened.
    c.session.doc_mut().document.timeline_mut(&anim_scene::TimelineId::Scene(0)).unwrap().layers[0].locked = true;
    let result = c.ok("draw_rectangle", json!({ "x": 1, "y": 1, "width": 50, "height": 50 }));
    assert_eq!(result["applied_commands"], 0);
    assert!(c.stage().is_empty());
}

#[test]
fn freehand_strokes_and_brush_pressure() {
    let mut c = Client::new();
    let points: Vec<Value> = (0..40).map(|i| json!([20 + i * 6, 100.0 + (f64::from(i) * 0.3).sin() * 30.0])).collect();
    c.ok("draw_freehand", json!({ "points": points, "stroke": { "width": 3 } }));
    let pressure: Vec<f64> = (0..40).map(|i| f64::from(i) / 40.0).collect();
    c.ok("draw_freehand", json!({ "points": points, "tool": "brush", "pressure": pressure, "fill": "#cc00cc", "object_drawing": true }));
    let elements = c.stage();
    assert_eq!(elements.len(), 2);
    assert_eq!(elements[1]["fills"], json!(["#CC00CC"]));
}

fn arb_json() -> BoxedStrategy<Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        (-1000.0_f64..1000.0).prop_map(|f| json!(f)),
        proptest::string::string_regex("[a-z#0-9]{0,8}").unwrap().prop_map(Value::from),
    ];
    let key = prop_oneof![
        Just("x"),
        Just("y"),
        Just("width"),
        Just("height"),
        Just("points"),
        Just("dx"),
        Just("dy"),
        Just("sx"),
        Just("degrees"),
        Just("fill"),
        Just("stroke"),
        Just("text"),
        Just("d"),
        Just("tool"),
        Just("size"),
        Just("mode"),
        Just("layer"),
        Just("frame"),
        Just("symbol"),
    ];
    let points = proptest::collection::vec((-100.0_f64..600.0, -100.0_f64..500.0).prop_map(|(x, y)| json!([x, y])), 0..6).prop_map(Value::from);
    let value = prop_oneof![3 => leaf, 1 => points];
    proptest::collection::vec((key, value), 0..6).prop_map(|entries| Value::Object(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())).boxed()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    #[test]
    fn random_arguments_never_panic_and_failures_change_nothing(calls in proptest::collection::vec((0_usize..40, arb_json()), 1..12)) {
        let mut c = Client::new();
        let names: Vec<String> = c.session.registry().iter().filter(|s| s.mcp).map(|s| anim_session::tool_name(s.id)).collect();
        for (index, args) in calls {
            let name = &names[index % names.len()];
            let before = c.editor().document.clone();
            let reply = c.rpc("tools/call", json!({ "name": name, "arguments": args }));
            if reply["result"]["isError"] == json!(true) {
                prop_assert_eq!(&c.editor().document, &before, "{} left a partial edit", name);
            }
        }
        prop_assert!(serde_json::to_string(&c.editor().document).is_ok());
    }
}
